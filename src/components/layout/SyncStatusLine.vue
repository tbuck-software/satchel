<script setup lang="ts">
/**
 * A quiet connection summary, with progress while older mail is imported.
 *
 * It stays silent while `ConnectionBanner` is up. A standing backend failure is
 * the same fact in a stronger, actionable form, and a local-first client that
 * says it twice in two tones reads as two problems (STYLEGUIDE.md § Error
 * States). The precedence lives in `connection.syncStatus`, so every mount
 * point of this line tells the same story.
 *
 * `status`, never `alert`: being offline is a state the user usually caused,
 * not a failure that interrupts them.
 */
import { computed, defineAsyncComponent, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useConnectionStore } from '@/stores/connection';
import { useOutboxStore } from '@/stores/outbox';
import AppBanner from '@/components/ui/AppBanner.vue';
import AppButton from '@/components/ui/AppButton.vue';
import CardButton from '@/components/ui/CardButton.vue';
import ProgressBar from '@/components/ui/ProgressBar.vue';
import { useSyncProgress } from '@/composables/useSyncProgress';
import { formatListTime } from '@/utils/listTime';

const props = withDefaults(defineProps<{
  /** Drop the resting "last synced" sentence, for places that are not a footer. */
  hideIdle?: boolean;
  /** Desktop sidebar: report progress while failures retain their existing affordances. */
  progressOnly?: boolean;
  /**
   * Show failed items as a banner with a way in, for the phone, where this
   * line is the only sync control on screen. Bare red text there read as
   * leaked debug output.
   */
  bannerFailures?: boolean;
}>(), { hideIdle: false, bannerFailures: false, progressOnly: false });

const SyncProgressDetails = defineAsyncComponent(() => import('./SyncProgressDetails.vue'));
const progressOpen = ref(false);
const connection = useConnectionStore();
const progress = useSyncProgress();
const slowSync = ref(false);
// Routine polls should not make the sidebar footer jump in and out.
watch(() => props.progressOnly && connection.syncing && !progress.history.value, (defer, _previous, onCleanup) => {
  slowSync.value = false;
  if (!defer) return;
  const timer = setTimeout(() => { slowSync.value = true; }, 3000);
  onCleanup(() => clearTimeout(timer));
}, { immediate: true });
let progressTrigger: HTMLElement | null = null;
function openProgress(event: MouseEvent) {
  progressTrigger = event.currentTarget as HTMLElement;
  progressTrigger.focus();
  progressOpen.value = true;
}

function closeProgress() {
  progressOpen.value = false;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const active = document.activeElement;
    if (active && active !== document.body && !active.closest('.sheet-root')) return;
    const target = progressTrigger?.isConnected ? progressTrigger : document.querySelector<HTMLElement>('.sync-button');
    target?.focus({ preventScroll: true });
  }));
}

const SyncFailures = defineAsyncComponent(() => import('./SyncFailures.vue'));
const failuresOpen = ref(false);
let opener: HTMLElement | null = null;
function openFailures(event: MouseEvent) {
  opener = event.currentTarget as HTMLElement;
  opener.focus();
  failuresOpen.value = true;
}
/*
 * The sheet hands the focus back to what opened it, but on a busy page that
 * hand-back can miss and the focus stays in the leaving sheet, then drops to
 * <body>. Then this line's own button takes it.
 */
function closeFailures() {
  failuresOpen.value = false;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const active = document.activeElement;
    // Still on <body>, or still inside the sheet that is leaving.
    if (active && active !== document.body && !active.closest('.sheet-root')) return;
    const button = opener?.isConnected ? opener : root.value?.querySelector<HTMLElement>('[aria-haspopup="dialog"]');
    button?.focus({ preventScroll: true });
  }));
}
const root = ref<HTMLElement | null>(null);
const outbox = useOutboxStore();
const failureCount = computed(() => props.progressOnly ? 0 : outbox.attentionCount);
const { t } = useI18n();
const progressTitle = computed(() => [progress.counts.value, progress.phase.value, t('app.sync.progress.details')].filter(Boolean).join(' · '));

const visible = computed(() => {
  if (connection.syncStatus === 'error') return false;
  if (props.progressOnly) return progress.history.value || slowSync.value;
  return !(props.hideIdle && connection.syncStatus === 'idle' && !progress.history.value);
});

const label = computed(() => {
  if (connection.syncStatus === 'offline') return t('app.sync.status.offline');
  if (connection.syncStatus === 'syncing') return t('app.sync.status.syncing');
  if (!connection.lastSyncAt) return t('app.sync.status.never');
  return t('app.sync.status.lastSynced', { time: formatListTime(connection.lastSyncAt) });
});
</script>

<template>
  <div v-if="failureCount > 0 || visible || progressOpen" ref="root">
  <AppBanner
    v-if="failureCount > 0 && props.bannerFailures"
    class="sync-failure-banner"
    :title="t('app.sync.failures.label', { count: failureCount }, failureCount)"
  >
    <template #actions>
      <AppButton size="sm" variant="soft" aria-haspopup="dialog" :aria-expanded="failuresOpen" @click="openFailures">
        {{ t('app.sync.failures.review') }}
      </AppButton>
    </template>
  </AppBanner>
  <AppButton v-else-if="failureCount > 0" variant="link" class="sync-failure-status" aria-haspopup="dialog" :aria-expanded="failuresOpen" @click="openFailures">
    {{ t('app.sync.failures.label', { count: failureCount }, failureCount) }}
  </AppButton>
  <div v-else-if="visible && progress.active.value" class="sync-progress" role="status">
    <CardButton class="sync-progress__button" :title="progressTitle" aria-haspopup="dialog" :aria-expanded="progressOpen" @click="openProgress">
      <span class="sync-progress__line">
        <span class="sync-progress__label">{{ progress.label.value }}</span>
        <span v-if="progress.percent.value" class="sync-progress__percent">{{ progress.percent.value }}</span>
      </span>
      <ProgressBar v-if="progress.history.value && progress.total.value > 0" class="sync-progress__bar"
        :value="progress.done.value" :max="progress.total.value" :label="progress.label.value" :value-text="progress.counts.value" />
    </CardButton>
  </div>
  <p v-else-if="visible" class="sync-status" :class="`sync-status--${connection.syncStatus}`" role="status">
    <span class="sync-status__dot" aria-hidden="true" />
    <span class="sync-status__text">{{ label }}</span>
  </p>
  <SyncProgressDetails v-if="progressOpen" :open="progressOpen" @close="closeProgress" />
  <SyncFailures v-if="failureCount > 0 || failuresOpen" :open="failuresOpen" @close="closeFailures" />
  </div>
</template>

<style scoped>
/* One line and a hairline: the footer is shared with the agenda, and a fresh
   install can sit here for an hour. Counts and the current folder live in the
   tooltip and the details sheet. */
.sync-progress { padding: 0 var(--space-xs); }
.sync-progress__button { display: grid; gap: var(--space-2xs); width: 100%; min-width: 0; padding: var(--space-xs) var(--space-sm); border-radius: var(--radius-md); text-align: left; transition: background-color var(--duration-fast) var(--ease-out); }
.sync-progress__button:hover { background: var(--bg-hover); }
.sync-progress__line { display: flex; align-items: baseline; gap: var(--space-sm); min-width: 0; font-size: var(--font-size-xs); }
.sync-progress__label { flex: 1; min-width: 0; overflow: hidden; color: var(--text-secondary); text-overflow: ellipsis; white-space: nowrap; }
.sync-progress__percent { flex-shrink: 0; color: var(--text-tertiary); font-variant-numeric: tabular-nums; }
.sync-progress__bar { height: var(--space-2xs); }

.sync-failure-status { color: var(--accent-red-soft); padding: var(--space-xs) var(--space-sm); }
.sync-status {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  margin: 0;
  min-width: 0;
  padding: var(--space-xs) var(--space-sm);
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
}

.sync-status__dot {
  flex-shrink: 0;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--text-tertiary);
}

.sync-status__text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sync-status--offline .sync-status__dot {
  background: var(--accent-amber);
}

.sync-status--offline .sync-status__text {
  color: var(--accent-amber-soft);
  font-weight: var(--weight-medium);
}

.sync-status--syncing .sync-status__dot {
  background: var(--accent-primary);
  animation: sync-status-pulse 1.4s var(--ease-in-out) infinite;
}

@keyframes sync-status-pulse {
  50% {
    opacity: 0.3;
  }
}

@media (prefers-reduced-motion: reduce) {
  .sync-status--syncing .sync-status__dot {
    animation: none;
  }
}
</style>
