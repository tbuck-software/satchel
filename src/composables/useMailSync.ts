import { computed, ref } from 'vue';
import { useConnectionStore } from '@/stores/connection';
import { useAccountQuotaStore } from '@/stores/accountQuota';
import { useToast } from '@/composables/useToast';
import { backendRunImapSync } from '@/services/backend';
import { transportErrorCause } from '@/utils/transportLabels';
import { t } from '@/i18n/runtime';

// The sidebar and command palette share an in-flight manual request, including
// the interval before the backend publishes its first connection event.
const pending = ref(false);

export function useMailSync() {
  const connection = useConnectionStore();
  const toast = useToast();
  const syncing = computed(() => pending.value || connection.syncing);

  async function syncMail() {
    if (syncing.value || !connection.online) return;
    pending.value = true;
    try {
      const summary = await backendRunImapSync();
      connection.applySyncSummary(summary);
      // A manual sync may have read a fresh quota; the automatic path reloads too.
      void useAccountQuotaStore().load();
      // The run's own verdict replaces the banner's: a clean run clears a
      // failure the last automatic run left standing, a failing one is shown
      // there, where it names the account — not as a "done" toast.
      connection.reportSyncFailures(summary.failures);
      if (summary.failures.length === 0) toast.success(t('commands.syncDone'));
    } catch (error) {
      const cause = transportErrorCause(error);
      toast.error(cause ? t('commands.syncFailedCause', { cause }) : t('commands.syncFailed'));
    } finally {
      pending.value = false;
    }
  }

  return { syncing, syncMail };
}
