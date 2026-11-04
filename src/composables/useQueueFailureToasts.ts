import { watch } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useOutboxStore } from '@/stores/outbox';
import { useUiStore } from '@/stores/ui';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { useToast } from '@/composables/useToast';

/** Announce each failure transition once. The queue remains its durable home. */
export function useQueueFailureToasts() {
  const outbox = useOutboxStore();
  const toast = useToast();
  const router = useRouter();
  const { t } = useI18n();
  const ui = useUiStore();
  let previous = new Set<string>();
  // The phone keeps standing failures in a banner above the list. What was
  // already failed when the queue first came back is that banner's to show; a
  // toast on top only repeated it and covered the list.
  let standingSeeded = false;
  watch(
    () => [
      outbox.loadedOnce,
      outbox.items.filter((item) => ['failed', 'delivery_unknown', 'delivered'].includes(item.status))
        .map((item) => ({ id: `${item.id}:${item.status}`, send: item.action_type === 'send_message' })),
    ] as const,
    ([loadedOnce, failures]) => {
      if (ui.isMobile && !standingSeeded) {
        previous = new Set(failures.map((item) => item.id));
        if (loadedOnce) standingSeeded = true;
        return;
      }
      const fresh = failures.filter((item) => !previous.has(item.id));
      previous = new Set(failures.map((item) => item.id));
      // The Outbox lists these items itself; a toast pointing there from there
      // only covers the page it names.
      if (router.currentRoute.value.name === 'outbox') return;
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
