import {
  APP_DRAG_CHANNEL,
  APP_DRAG_TYPES,
  dragTypes,
  endAppDrag,
  startAppDrag,
  startedAppDrag,
  type AppDrag,
  type AppDragKind,
  type RecipientDragSource,
} from './appDrag';

/*
 * The drop side of `appDrag.ts`, for the composer and for drag sources the
 * startup bundle does not hold.
 */

/** A drag whose end nobody announced (its window closed) stops counting after this. */
const REMOTE_TTL_MS = 2 * 60_000;

let remote: { drag: AppDrag; at: number } | null = null;

// Listen from the first import: a drag can start in another window any time.
try {
  const channel = new BroadcastChannel(APP_DRAG_CHANNEL);
  channel.onmessage = (event: MessageEvent<AppDrag | null>) => {
    remote = event.data ? { drag: event.data, at: Date.now() } : null;
  };
  // Node (the tests) would otherwise stay alive for the listener.
  (channel as { unref?: () => void }).unref?.();
} catch {
  // No channel: drags from other windows rely on their custom types alone.
}

/** Files and other applications' drags are never ours, whatever is remembered. */
function mayBeAppDrag(event: DragEvent): boolean {
  return dragTypes(event).every((type) => type === 'text/plain' || type.startsWith('application/x-satchel-'));
}

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

/** The drag in progress as this window knows it: its own, or one announced by another window. */
function known(event: DragEvent): AppDrag | null {
  if (!mayBeAppDrag(event)) return null;
  const own = startedAppDrag();
  if (own) return own;
  if (remote && Date.now() - remote.at > REMOTE_TTL_MS) remote = null;
  return remote?.drag ?? null;
}

function typedKind(event: DragEvent): AppDragKind | null {
  const types = dragTypes(event);
  return (Object.keys(APP_DRAG_TYPES) as AppDragKind[]).find((kind) => types.includes(APP_DRAG_TYPES[kind])) ?? null;
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
  remote = null;
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

/** For tests: forget everything without announcing it. */
export function resetAppDragForTests() {
  remote = null;
  endAppDrag();
}
