import { defineStore } from 'pinia';
import { ref } from 'vue';
import {
  backendCancelOutboxAction,
  backendListFailedSyncActions,
  backendDiscardFailedSyncAction,
  backendCommandsAvailable,
  backendListOutbox,
  backendRetryOutboxAction,
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

  async function load() {
    if (!backendCommandsAvailable()) return;
    loading.value = true;
    try {
      const [mail, moves] = await Promise.all([backendListOutbox(), backendListFailedSyncActions()]);
      items.value = [...moves, ...mail];
      error.value = null;
      connectionStore.clearBackendError('outbox');
    } catch (err) {
      // Never rethrows: callers are click handlers and `start()`, and the
      // rejection used to vanish into `Promise.allSettled` at bootstrap.
      // `items` stays untouched — a failed list says nothing about the queue.
      error.value = err instanceof Error ? err.message : String(err);
      connectionStore.reportBackendError('outbox', err);
    } finally {
      loading.value = false;
    }
  }

  async function retry(id: string) {
    await backendRetryOutboxAction(id);
    items.value = items.value.filter((i) => i.id !== id);
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
  async function start() {
    if (!backendCommandsAvailable()) return;
    // Events are a Tauri-only channel; the mock transport has no emitter.
    if (!backendSupportsNativeCommands()) {
      await load();
      return;
    }
    // Listeners first, then the initial load: a missed `outbox:failed` hides a
    // mail that never went out, a duplicate load costs one query.
    try {
      const mod = await import('@tauri-apps/api/event');
      unlisteners = await Promise.all(
        OUTBOX_EVENTS.map((eventName) => mod.listen(eventName, () => void load())),
      );
    } catch (err) {
      console.warn('outbox listener setup failed', err);
    }
    await load();
  }
  function stop() {
    for (const unlisten of unlisteners) unlisten();
    unlisteners = [];
  }

  return { items, loading, error, load, retry, cancel, start, stop };
});
