import {
  APP_DRAG_TYPES,
  dragTypes,
  endAppDrag,
  startAppDrag,
  startedAppDrag,
  type AppDrag,
  type AppDragKind,
  type RecipientDragSource,
} from './appDrag';
import { forgetRemoteAppDrag, remoteAppDrag } from './appDragChannel';

/*
 * The drop side of `appDrag.ts`, for the composer and for drag sources the
 * startup bundle does not hold.
 */

/** The payload as the drag carries it; `null` for anything malformed. */
function parseAppDrag(kind: AppDragKind, data: string): AppDrag | null {
  try {
    const value = JSON.parse(data) as AppDrag;
    const list: unknown = value?.kind !== kind ? null : value.kind === 'recipients' ? value.addresses : value.ids;
    const valid = Array.isArray(list) && list.length > 0 && list.every((entry) => typeof entry === 'string' && entry.trim());
    return valid ? value : null;
  } catch {
    return null;
  }
}

function typedKind(event: DragEvent): AppDragKind | null {
  const types = dragTypes(event);
  return (Object.keys(APP_DRAG_TYPES) as AppDragKind[]).find((kind) => types.includes(APP_DRAG_TYPES[kind])) ?? null;
}

/**
 * What this window remembers of the drag, if it is the one on the event. With
 * a Satchel type on it the memory only adds what `dragover` cannot read (a
 * chip's origin), and only for the same kind. Without one it stands in for
 * hidden types. A drag this window did not start through `startAppDrag` (text
 * out of the Subject) has already cleared it (see `appDrag.ts`).
 */
function known(event: DragEvent): AppDrag | null {
  const kind = typedKind(event);
  const drag = startedAppDrag() ?? remoteAppDrag();
  if (kind) return drag?.kind === kind ? drag : null;
  // Hidden types leave at most the addresses' text; files or another
  // application's formats mean the drag is not ours.
  return dragTypes(event).every((type) => type === 'text/plain') ? drag : null;
}

/** Which of Satchel's drags this is; readable during `dragover`. */
export function appDragKind(event: DragEvent): AppDragKind | null {
  return typedKind(event) ?? known(event)?.kind ?? null;
}

/**
 * The drag in progress as far as it is known before the drop, when `getData`
 * is still empty: it carries a chip's origin.
 */
export function pendingAppDrag(event: DragEvent): AppDrag | null {
  const drag = known(event);
  return drag && drag.kind === appDragKind(event) ? drag : null;
}

/** The dragged payload; complete on `drop`. */
export function readAppDrag(event: DragEvent): AppDrag | null {
  const kind = typedKind(event);
  return (kind && parseAppDrag(kind, event.dataTransfer?.getData(APP_DRAG_TYPES[kind]) ?? '')) || known(event);
}

/** The drop took the drag: forget it here and in the window it came from. */
export function finishAppDrag() {
  forgetRemoteAppDrag();
  endAppDrag();
}

/**
 * Recipients (a contact, a chip). Another application gets the addresses as
 * text; a contact can only be copied, a chip also moved.
 */
export function startRecipientDrag(event: DragEvent, addresses: string[], source: RecipientDragSource | null) {
  startAppDrag(event, { kind: 'recipients', addresses, source });
  const transfer = event.dataTransfer;
  if (!transfer) return;
  transfer.setData('text/plain', addresses.join(', '));
  if (!source) transfer.effectAllowed = 'copy';
}

export function startMessageDrag(event: DragEvent, messageIds: string[]) {
  startAppDrag(event, { kind: 'messages', ids: messageIds });
}

/** A drag past this is over, whether or not its end arrived. */
const DRAG_WATCHDOG_MS = 60_000;

/**
 * Calls `recover` when a drag's end never arrives: WKWebView may not deliver
 * `dragend` once the source has unmounted. While a drag runs the page gets
 * neither pointer nor key events, so the first of either means it is over.
 * Returns the function that stops watching.
 */
export function watchDragEnd(recover: () => void, timeoutMs = DRAG_WATCHDOG_MS): () => void {
  const options = { capture: true } as const;
  const timer = setTimeout(fire, timeoutMs);
  window.addEventListener('pointermove', fire, options);
  window.addEventListener('keydown', fire, options);
  function stop() {
    clearTimeout(timer);
    window.removeEventListener('pointermove', fire, options);
    window.removeEventListener('keydown', fire, options);
  }
  function fire() {
    stop();
    finishAppDrag();
    recover();
  }
  return stop;
}

/** For tests: forget everything. */
export function resetAppDragForTests() {
  finishAppDrag();
}
