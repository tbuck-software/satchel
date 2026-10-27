import { watch } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useOutboxStore } from '@/stores/outbox';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { useToast } from '@/composables/useToast';

/** Announce each failure transition once. The queue remains its durable home. */
export function useQueueFailureToasts() {
  const outbox = useOutboxStore();
  const toast = useToast();
  const router = useRouter();
  const { t } = useI18n();
  let previous = new Set<string>();
  watch(
    () => outbox.items.filter((item) => ['failed', 'delivery_unknown', 'delivered'].includes(item.status))
      .map((item) => ({ id: `${item.id}:${item.status}`, send: item.action_type === 'send_message' })),
    (failures) => {
      const fresh = failures.filter((item) => !previous.has(item.id));
      previous = new Set(failures.map((item) => item.id));
      for (const send of [true, false]) {
        const count = fresh.filter((item) => item.send === send).length;
        if (count === 0) continue;
        toast.error(t(send ? 'app.outbox.needsAttention' : 'app.sync.mutationsFailed', { count }, count), {
          timeout: 7000,
          groupKey: send ? 'outbox-failures' : 'sync-action-failures',
          action: send ? {
            label: t('nav.folders.outbox'),
            handler: () => router.push({ name: 'outbox' }),
          } : {
            label: t('common.retry'),
            handler: async () => {
              try {
                const retried = await outbox.retryFailedSyncActions();
                toast.info(t('app.sync.retryQueued', { count: retried }, retried));
              } catch (error) {
                toast.error(t('app.sync.retryFailed', { error: backendErrorLabel(error) }));
              }
            },
          },
        });
      }
    },
    { immediate: true },
  );
}
