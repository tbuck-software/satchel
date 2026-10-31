<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendInlineImageCacheStatus,
  backendStartInlineImageBackfill,
  backendStopInlineImageBackfill,
  backendSupportsNativeCommands,
  translateBackendError,
  type BackendInlineCacheProgress,
  type BackendInlineCacheStatus,
} from '@/services/backend';
import { listenToBackend } from '@/services/appEvents';
import { formatBytes } from '@/utils/formatBytes';
import AppButton from '@/components/ui/AppButton.vue';

const { locale, t } = useI18n();
const status = ref<BackendInlineCacheStatus | null>(null);
const progress = ref<BackendInlineCacheProgress | null>(null);
const error = ref('');
const busy = ref(false);
let unlisten: (() => void) | null = null;

const native = backendSupportsNativeCommands();

onMounted(async () => {
  if (!native) return;
  await refresh();
  unlisten = await listenToBackend<BackendInlineCacheProgress>('inline-cache:progress', (payload) => {
    progress.value = payload;
    // The counters in the header only move once a run ends; refreshing then
    // keeps "fehlen noch" honest without polling throughout.
    if (!payload.running) void refresh();
  });
});

onBeforeUnmount(() => {
  unlisten?.();
});

async function refresh() {
  try {
    const next = await backendInlineImageCacheStatus();
    status.value = next;
    progress.value = next.progress;
    error.value = '';
  } catch (err) {
    error.value = errorText(err);
  }
}

async function start() {
  busy.value = true;
  error.value = '';
  try {
    await backendStartInlineImageBackfill();
  } catch (err) {
    error.value = errorText(err);
  } finally {
    busy.value = false;
  }
}

async function stop() {
  try {
    await backendStopInlineImageBackfill();
  } catch (err) {
    error.value = errorText(err);
  }
}

/** Rust names its failures by message key; a provider's own wording stays as it is. */
function errorText(err: unknown): string {
  return translateBackendError(err) ?? (err instanceof Error ? err.message : String(err));
}

const lastError = computed(() => {
  const raw = progress.value?.last_error;
  return raw ? errorText(raw) : '';
});

const running = computed(() => progress.value?.running === true);
const missing = computed(() => status.value?.missing ?? 0);

const handled = computed(() => (progress.value?.done ?? 0) + (progress.value?.failed ?? 0));
const percent = computed(() => {
  const total = progress.value?.total ?? 0;
  if (total <= 0) return 0;
  return Math.min(100, Math.round((handled.value / total) * 100));
});

const paused = computed(() => (progress.value?.paused_seconds ?? 0) > 0);

const etaLabel = computed(() => {
  const seconds = progress.value?.eta_seconds ?? 0;
  if (!running.value || seconds <= 0) return '';
  return t('settings.inlineImages.eta', {
    duration: formatDuration(seconds),
    time: clockIn(seconds),
  });
});

/**
 * Estimate shown before the run starts. It counts only the pacing delay, so it
 * is a floor rather than a forecast — saying so beats a number that quietly
 * turns out to be half the truth.
 */
const upfrontLabel = computed(() => {
  const seconds = status.value?.estimated_seconds ?? 0;
  if (running.value || missing.value === 0 || seconds <= 0) return '';
  return t('settings.inlineImages.upfront', {
    duration: formatDuration(seconds),
    time: clockIn(seconds),
  });
});

function clockIn(seconds: number): string {
  return new Date(Date.now() + seconds * 1000)
    .toLocaleTimeString(locale.value, { hour: '2-digit', minute: '2-digit' });
}

const finishedLabel = computed(() => {
  const reason = progress.value?.finished_reason;
  if (!reason || running.value) return '';
  const done = progress.value?.done ?? 0;
  const failed = progress.value?.failed ?? 0;
  const tail = failed > 0
    ? t('settings.inlineImages.failedTail', { count: formatCount(failed) })
    : '';
  if (reason === 'cancelled') {
    return t('settings.inlineImages.cancelled', { count: formatCount(done), tail });
  }
  if (reason === 'stalled') {
    return t('settings.inlineImages.stalled', { count: formatCount(done), tail });
  }
  if (reason === 'error') return t('settings.inlineImages.startFailed');
  return t('settings.inlineImages.finished', { count: formatCount(done), tail });
});

function formatCount(value: number): string {
  return value.toLocaleString(locale.value);
}

function formatDuration(seconds: number): string {
  if (seconds < 90) {
    const count = Math.max(1, Math.round(seconds));
    return t('settings.inlineImages.duration.seconds', count);
  }
  const minutes = Math.round(seconds / 60);
  if (minutes < 90) return t('settings.inlineImages.duration.minutes', minutes);
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0
    ? t('settings.inlineImages.duration.hours', hours)
    : t('settings.inlineImages.duration.hoursMinutes', { hours, minutes: rest });
}
</script>

<template>
  <section class="backfill">
    <header>
      <h3>{{ $t('settings.inlineImages.title') }}</h3>
      <p>{{ $t('settings.inlineImages.description') }}</p>
    </header>

    <p v-if="!native" class="hint">{{ $t('settings.inlineImages.desktopOnly') }}</p>

    <template v-else-if="status">
      <div class="counts">
        <div>
          <strong>{{ formatCount(missing) }}</strong>
          <span>{{ $t('settings.inlineImages.missingLocally', { size: formatBytes(status.missing_bytes) }) }}</span>
        </div>
        <div>
          <strong>{{ formatCount(status.cached) }}</strong>
          <span>{{ $t('settings.inlineImages.alreadyStored') }}</span>
        </div>
      </div>

      <div v-if="running || progress?.finished_reason" class="run">
        <div class="bar" role="progressbar" :aria-valuenow="percent" aria-valuemin="0" aria-valuemax="100">
          <span :style="{ width: `${percent}%` }" :class="{ paused }" />
        </div>
        <p class="run-line">
          <span>{{ $t('settings.inlineImages.progressOf', { done: formatCount(handled), total: formatCount(progress?.total ?? 0) }) }}</span>
          <span v-if="paused" class="paused-note">
            {{ $t('settings.inlineImages.throttled', { duration: formatDuration(progress?.paused_seconds ?? 0) }) }}
          </span>
          <span v-else-if="etaLabel">{{ etaLabel }}</span>
          <span v-else-if="finishedLabel">{{ finishedLabel }}</span>
        </p>
      </div>

      <p v-if="missing === 0 && !running" class="hint ok">
        {{ $t('settings.inlineImages.allPresent') }}
      </p>
      <p v-else-if="upfrontLabel" class="hint">{{ upfrontLabel }}</p>

      <div class="actions">
        <AppButton v-if="running" variant="soft" size="sm" @click="stop">{{ $t('settings.inlineImages.stop') }}</AppButton>
        <AppButton
          v-else
          variant="primary"
          size="sm"
          :disabled="busy || missing === 0"
          @click="start"
        >
          {{ $t('settings.inlineImages.startNow') }}
        </AppButton>
        <span class="note">{{ $t('settings.inlineImages.note') }}</span>
      </div>

      <p v-if="lastError" class="hint error">{{ lastError }}</p>
    </template>

    <p v-if="error" class="hint error">{{ error }}</p>
  </section>
</template>

<style scoped>
.backfill {
  display: grid;
  gap: 14px;
}

header h3 {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  letter-spacing: -0.01em;
}

header p {
  margin: 5px 0 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  line-height: 1.45;
}

.counts {
  display: flex;
  gap: 28px;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  padding: 14px 16px;
}

.counts div {
  display: grid;
  gap: 2px;
}

.counts strong {
  color: var(--text-primary);
  font-size: var(--font-size-xl);
  font-weight: var(--weight-semibold);
  font-variant-numeric: tabular-nums;
}

.counts span {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.run {
  display: grid;
  gap: 7px;
}

.bar {
  overflow: hidden;
  height: 6px;
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
}

.bar span {
  display: block;
  height: 100%;
  border-radius: var(--radius-full);
  background: var(--accent-primary);
  transition: width var(--duration-fast) var(--ease-out);
}

.bar span.paused {
  background: var(--accent-amber);
}

.run-line {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 10px;
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.paused-note {
  color: var(--accent-amber);
}

.actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.note {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  line-height: 1.4;
}

.hint {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.hint.ok {
  color: var(--accent-green);
}

.hint.error {
  color: var(--accent-red);
}
</style>
