<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendGetSpamFilterStatus,
  backendSetSpamFilterMode,
  type BackendSpamFilterMode,
  type BackendSpamFilterStatus,
} from '@/services/backend';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';

const { t, n } = useI18n();
const status = ref<BackendSpamFilterStatus | null>(null);
const error = ref('');

const mode = computed<BackendSpamFilterMode>(() => status.value?.mode ?? 'mark');

const modeOptions = computed(() => [
  { value: 'off', label: t('settings.spamFilter.modeOff'), title: t('settings.spamFilter.modeOffTitle') },
  { value: 'mark', label: t('settings.spamFilter.modeMark'), title: t('settings.spamFilter.modeMarkTitle') },
  { value: 'move', label: t('settings.spamFilter.modeMove'), title: t('settings.spamFilter.modeMoveTitle') },
]);

const modeHint = computed(() => ({
  off: t('settings.spamFilter.modeOffHint'),
  mark: t('settings.spamFilter.modeMarkHint'),
  move: t('settings.spamFilter.modeMoveHint'),
})[mode.value]);

const trainingLine = computed(() => {
  const current = status.value;
  if (!current) return '';
  return t('settings.spamFilter.trained', {
    spam: n(current.spam_messages),
    ham: n(current.ham_messages),
  });
});

/** What stands between the filter and a verdict, or between Mark and Move. */
const readinessLine = computed(() => {
  const current = status.value;
  if (!current) return '';
  if (current.bootstrap_pending) return t('settings.spamFilter.bootstrapping');
  if (!current.ready) {
    return t('settings.spamFilter.needsMore', {
      spam: n(current.spam_needed),
      ham: n(current.ham_needed),
    });
  }
  if (mode.value === 'move' && !current.move_ready) {
    return t('settings.spamFilter.moveNeedsMore', {
      spam: n(current.move_spam_needed),
      ham: n(current.move_ham_needed),
    });
  }
  return t('settings.spamFilter.judging');
});

async function load() {
  try {
    status.value = await backendGetSpamFilterStatus();
    error.value = '';
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  }
}

onMounted(load);

async function setMode(value: string) {
  const next = value as BackendSpamFilterMode;
  const previous = status.value;
  if (previous) status.value = { ...previous, mode: next };
  try {
    await backendSetSpamFilterMode(next);
    error.value = '';
  } catch (err) {
    status.value = previous;
    error.value = err instanceof Error ? err.message : String(err);
  }
}
</script>

<template>
  <section>
    <h3 class="settings-page-title">{{ $t('settings.spamFilter.title') }}</h3>
    <p class="hint">{{ $t('settings.spamFilter.purpose') }}</p>

    <div class="setting-group">
      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.spamFilter.modeLabel') }}</span>
          <span class="setting-hint">{{ modeHint }}</span>
        </div>
        <SegmentedControl
          :options="modeOptions"
          :model-value="mode"
          :aria-label="$t('settings.spamFilter.modeAria')"
          @update:model-value="setMode"
        />
      </div>
      <div v-if="status" class="setting-row setting-row--single">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.spamFilter.trainingLabel') }}</span>
          <span class="setting-hint" data-testid="spam-filter-training">{{ trainingLine }}</span>
          <span class="setting-hint" data-testid="spam-filter-readiness">{{ readinessLine }}</span>
        </div>
      </div>
    </div>

    <p class="hint">{{ $t('settings.spamFilter.providers') }}</p>
    <p v-if="error" class="status">{{ error }}</p>
  </section>
</template>

<style scoped src="./settingsForms.css"></style>

<style scoped src="./settingsRows.css"></style>
