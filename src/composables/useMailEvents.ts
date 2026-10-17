import { getCurrentInstance, onBeforeUnmount } from 'vue';
import { backendSupportsNativeCommands } from '@/services/backend';

/**
 * Subscribe to the backend's mail events from anywhere in the UI.
 *
 * Two events matter to a view that shows mail:
 *
 * - `mail:incoming` — a genuinely new inbox message was ingested. Note the
 *   backend only publishes it for mail that is in an inbox folder, not from
 *   the account itself, and less than 15 minutes old (`new_mail_notify_age`
 *   in `sync/message_ingest.rs`), so it is a "notify the user" signal rather
 *   than a complete change feed.
 * - `thread:updated` — a thread changed outside the UI. Today that is a quick
 *   action on a macOS notification (`utils/mac_notify.rs`), which archives,
 *   trashes or marks read directly in the database. Until this composable
 *   existed nothing in the frontend listened, so those actions changed state
 *   behind the stores' backs and the open window kept showing the old row —
 *   despite a comment in `mac_notify.rs` claiming the stores handled it.
 *
 * Listeners are torn down with the calling component. Outside a component
 * setup() the returned disposer must be called by hand.
 */
export interface MailEventHandlers {
  onIncoming?: () => void;
  onThreadsUpdated?: (threadIds: string[]) => void;
  onResync?: () => void;
}

export function useMailEvents(handlers: MailEventHandlers): () => void {
  let disposers: Array<() => void> = [];
  let disposed = false;

  function dispose() {
    disposed = true;
    for (const off of disposers) off();
    disposers = [];
  }

  if (backendSupportsNativeCommands()) {
    void (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const registered: Array<() => void> = [];

        if (handlers.onIncoming) {
          registered.push(await listen('mail:incoming', () => handlers.onIncoming?.()));
        }
        if (handlers.onThreadsUpdated) {
          registered.push(
            await listen<{ thread_id?: string; thread_ids?: string[] }>('thread:updated', event => {
              const ids = event.payload?.thread_ids ?? (event.payload?.thread_id ? [event.payload.thread_id] : []);
              if (ids.length) handlers.onThreadsUpdated?.(ids);
            }),
          );
        }
        if (handlers.onResync) {
          registered.push(await listen('mail:resync', () => handlers.onResync?.()));
        }

        // The component may have unmounted while `listen()` was in flight —
        // without this the listener would outlive it and fire into a dead view.
        if (disposed) {
          for (const off of registered) off();
          return;
        }
        disposers = registered;
      } catch (err) {
        console.warn('mail event subscription failed', err);
      }
    })();
  }

  if (getCurrentInstance()) {
    onBeforeUnmount(dispose);
  }

  return dispose;
}
