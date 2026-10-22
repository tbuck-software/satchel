import { computed, ref } from 'vue';
import { useConnectionStore } from '@/stores/connection';
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
      connection.applySyncSummary(await backendRunImapSync());
      toast.success(t('commands.syncDone'));
    } catch (error) {
      const cause = transportErrorCause(error);
      toast.error(cause ? t('commands.syncFailedCause', { cause }) : t('commands.syncFailed'));
    } finally {
      pending.value = false;
    }
  }

  return { syncing, syncMail };
}
