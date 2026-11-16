import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useConnectionStore } from '@/stores/connection';

/**
 * Whole percent, rounded down: 99.6 % of a mailbox is not "100 %", and a
 * bar that reads full while work remains looks stuck.
 */
export function progressPercent(done: number, total: number): number {
  if (total <= 0) return 0;
  return Math.floor(Math.min(1, Math.max(0, done / total)) * 100) / 100;
}

/** Shared progress projection and copy for the sidebar, phone and details. */
export function useSyncProgress() {
  const connection = useConnectionStore();
  const { t, n } = useI18n();
  const importing = computed(() => connection.syncProgress.filter(row => row.backfill_remaining > 0));
  const remaining = computed(() => importing.value.reduce((sum, row) => sum + row.backfill_remaining, 0));
  const total = computed(() => connection.syncProgress.reduce((sum, row) => sum + Math.max(row.backfill_remaining, row.backfill_total), 0));
  const done = computed(() => Math.max(0, total.value - remaining.value));
  const history = computed(() => remaining.value > 0);
  const active = computed(() => connection.syncing || history.value);
  const label = computed(() => {
    if (history.value && !connection.online) return t('app.sync.progress.paused');
    return history.value ? t('app.sync.progress.history') : t('app.sync.status.syncing');
  });
  const counts = computed(() => total.value > 0
    ? t('app.sync.progress.counts', { done: n(done.value), total: n(total.value) }) : '');
  const percent = computed(() => history.value && total.value > 0
    ? n(progressPercent(done.value, total.value), 'percent') : '');
  const current = computed(() => connection.syncProgress.find(row => row.phase === 'folder')
    ?? connection.syncProgress.find(row => row.phase === 'starting'));
  const phase = computed(() => {
    if (!connection.online) return t('app.sync.progress.paused');
    if (!connection.syncing) return t('app.sync.progress.waiting');
    if (current.value?.folder) return t('app.sync.progress.folder', { folder: current.value.folder });
    if (current.value?.phase === 'starting') return t('app.sync.progress.preparing');
    return t('app.sync.progress.waiting');
  });
  return { active, history, label, counts, percent, phase, done, total };
}
