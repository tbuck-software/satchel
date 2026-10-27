import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import {
  backendCancelOutboxAction,
  backendListFailedSyncActions,
  backendDiscardFailedSyncAction,
  backendCommandsAvailable,
  backendListOutbox,
  backendRetryOutboxAction,
  backendRetryFailedSyncActions,
  backendSupportsNativeCommands,
  type BackendOutboxItem,
} from '@/services/backend';
import { useConnectionStore } from '@/stores/connection';

/**
 * The queue events that make the outbox reload.
 *
 * A named constant rather than an inline array because the event parity scan
 * (`src/architecture/eventCoverage.test.ts`) reads it: these five are the only
 * listeners in the tree that are not a literal at the `listen(` call.
 */
export const OUTBOX_EVENTS = [
  'outbox:failed',
  'outbox:queued',
  'outbox:retry',
  'outbox:scheduled',
  'outbox:sent',
] as const;

export const useOutboxStore = defineStore('outbox', () => {
  const items = ref<BackendOutboxItem[]>([]);
  const loading = ref(false);
  /** Raw cause of the last failed list, so the view can tell empty from broken. */
  const error = ref<string | null>(null);
  const connectionStore = useConnectionStore();
  const failedSendCount = computed(() => items.value.filter((item) =>
    item.action_type === 'send_message' && ['failed', 'delivered', 'delivery_unknown'].includes(item.status),
  ).length);
  const failedChangeCount = computed(() => items.value.filter((item) =>
    item.action_type !== 'send_message' && item.status === 'failed',
  ).length);
  // Queue events arrive before the periodic count snapshot. Keep attention
  // reachable during that gap, including after a short-lived receipt expires.
  const attentionCount = computed(() => Math.max(connectionStore.failedSends, failedSendCount.value)
    + Math.max(connectionStore.failedRemoteMutations, failedChangeCount.value));

  // Five events trigger a reload; only the newest request may publish.
  let loadRequestId = 0;

  async function load() {
    if (!backendCommandsAvailable()) return;
    const requestId = ++loadRequestId;
    loading.value = true;
    try {
      const [mail, moves] = await Promise.all([backendListOutbox(), backendListFailedSyncActions()]);
      if (requestId !== loadRequestId) return;
      items.value = [...moves, ...mail];
      error.value = null;
      connectionStore.clearBackendError('outbox');
    } catch (err) {
      if (requestId !== loadRequestId) return;
      // Never rethrows: callers are click handlers and `start()`, and the
      // rejection used to vanish into `Promise.allSettled` at bootstrap.
      // `items` stays untouched — a failed list says nothing about the queue.
      error.value = err instanceof Error ? err.message : String(err);
      connectionStore.reportBackendError('outbox', err);
    } finally {
      if (requestId === loadRequestId) loading.value = false;
    }
  }

  async function retry(id: string) {
    await backendRetryOutboxAction(id);
    items.value = items.value.filter((i) => i.id !== id);
  }

  async function retryFailedSyncActions() {
    const count = await backendRetryFailedSyncActions();
    await Promise.all([load(), connectionStore.refresh()]);
    return count;
  }

  async function cancel(id: string) {
    if (items.value.find((item) => item.id === id)?.action_type === 'remote_mutation') {
      await backendDiscardFailedSyncAction(id);
      await connectionStore.refresh();
    } else {
      await backendCancelOutboxAction(id);
    }
    items.value = items.value.filter((i) => i.id !== id);
  }

  let unlisteners: Array<() => void> = [];
  // `stop()` bumps the generation, so a `start()` still awaiting its listeners
  // releases them instead of leaking them past the stop.
  let generation = 0;
  let started = false;
  async function start() {
    if (started || !backendCommandsAvailable()) return;
    started = true;
    const session = ++generation;
    // Events are a Tauri-only channel; the mock transport has no emitter.
    if (!backendSupportsNativeCommands()) {
      await load();
      return;
    }
    // Listeners first, then the initial load: a missed `outbox:failed` hides a
    // mail that never went out, a duplicate load costs one query.
    try {
      const mod = await import('@tauri-apps/api/event');
      if (session !== generation) return;
      const registered = await Promise.allSettled(
        OUTBOX_EVENTS.map((eventName) => mod.listen(eventName, () => {
          if (session === generation) void load();
        })),
      );
      const released = registered.flatMap((result) => result.status === 'fulfilled' ? [result.value] : []);
      if (session !== generation) {
        released.forEach((unlisten) => unlisten());
        return;
      }
      unlisteners = released;
      const failed = registered.find((result) => result.status === 'rejected');
      if (failed) console.warn('outbox listener setup failed', failed.reason);
    } catch (err) {
      console.warn('outbox listener setup failed', err);
    }
    if (session === generation) await load();
  }
  function stop() {
    generation += 1;
    started = false;
    for (const unlisten of unlisteners) unlisten();
    unlisteners = [];
  }

  return { items, loading, error, failedSendCount, failedChangeCount, attentionCount, load, retry, retryFailedSyncActions, cancel, start, stop };
});
