import { DRAG_TTL_MS, connectAppDragChannel, type AppDrag } from './appDrag';

/*
 * The app's windows tell each other about the drag in progress: one channel
 * per window, for as long as it lives. A drop in another window resolves
 * through it should the webview hide the drag's custom types. Loaded with the
 * drop side; from then on this window's own drags are announced too.
 */

const APP_DRAG_CHANNEL = 'satchel:app-drag';

let remote: { drag: AppDrag; at: number } | null = null;

const channel = openChannel();

function openChannel(): BroadcastChannel | null {
  try {
    const opened = new BroadcastChannel(APP_DRAG_CHANNEL);
    opened.onmessage = (event: MessageEvent<AppDrag | null>) => {
      remote = event.data ? { drag: event.data, at: Date.now() } : null;
    };
    // Node (the tests) would otherwise stay alive for the listener.
    (opened as { unref?: () => void }).unref?.();
    return opened;
  } catch {
    // Without it another window's drop relies on the custom types alone.
    return null;
  }
}

function postAppDrag(drag: AppDrag | null) {
  try {
    channel?.postMessage(drag);
  } catch {
    // A closed channel only costs the other windows' fallback.
  }
}

connectAppDragChannel(postAppDrag);

/** The drag another window announced, unless it is long over. */
export function remoteAppDrag(): AppDrag | null {
  if (remote && Date.now() - remote.at > DRAG_TTL_MS) remote = null;
  return remote?.drag ?? null;
}

export function forgetRemoteAppDrag() {
  remote = null;
}

// As for this window's own drags (`appDrag.ts`): a drag starting here, a
// drag ending here and a drop here, once read, end the memory of it.
if (typeof window !== 'undefined') {
  window.addEventListener('dragstart', forgetRemoteAppDrag, true);
  window.addEventListener('dragend', forgetRemoteAppDrag, true);
  window.addEventListener('drop', () => setTimeout(forgetRemoteAppDrag, 0), true);
}
