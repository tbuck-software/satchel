/**
 * What Satchel itself drags: threads out of a list, single messages (a search
 * hit, one message of an open thread) and recipients (a contact, a chip out of
 * a recipient field).
 *
 * Each kind travels under its own MIME type, because during `dragover` a
 * target can read the type list but not the data. Folders take threads as a
 * move, the composer takes threads and messages as a copy (an `.eml` each)
 * and recipients into a field, so mail drags allow both effects and each
 * target names its own.
 *
 * The drag is also remembered for as long as it lasts and announced to the
 * app's other windows (`appDragTarget.ts` listens). WebKit shows custom types
 * only to a page of the origin that wrote them; should a webview hide them
 * anyway, a drop still resolves. The data on the drag stays the first source.
 *
 * This half is what drag sources need and sits in the startup bundle; the
 * composer's side lives in `appDragTarget.ts`.
 */
export const THREAD_DRAG_TYPE = 'application/x-satchel-thread-ids';
export const MESSAGE_DRAG_TYPE = 'application/x-satchel-message-ids';
export const RECIPIENT_DRAG_TYPE = 'application/x-satchel-recipients';
export const APP_DRAG_CHANNEL = 'satchel:app-drag';

export type RecipientField = 'to' | 'cc' | 'bcc';

/** Where a dragged recipient chip came from; a drop elsewhere in the same composer moves it. */
export interface RecipientDragSource {
  composer: string;
  field: RecipientField;
}

export type AppDrag =
  | { kind: 'threads'; ids: string[] }
  | { kind: 'messages'; ids: string[] }
  | { kind: 'recipients'; addresses: string[]; source: RecipientDragSource | null };

export type AppDragKind = AppDrag['kind'];

export const APP_DRAG_TYPES: Record<AppDragKind, string> = {
  threads: THREAD_DRAG_TYPE,
  messages: MESSAGE_DRAG_TYPE,
  recipients: RECIPIENT_DRAG_TYPE,
};

let started: AppDrag | null = null;

/** The drag this window started, until it ends. */
export function startedAppDrag() {
  return started;
}

function announce(drag: AppDrag | null) {
  started = drag;
  try {
    const channel = new BroadcastChannel(APP_DRAG_CHANNEL);
    channel.postMessage(drag);
    channel.close();
  } catch {
    // Without the channel only another window's fallback is missing.
  }
}

/** Puts `drag` on the event and announces it; a move or a copy, as the target decides. */
export function startAppDrag(event: DragEvent, drag: AppDrag) {
  const transfer = event.dataTransfer;
  if (!transfer) return;
  transfer.setData(APP_DRAG_TYPES[drag.kind], JSON.stringify(drag));
  transfer.effectAllowed = 'copyMove';
  announce(drag);
  event.target?.addEventListener('dragend', endAppDrag, { once: true });
}

/** Forgets the drag in progress, in every window. */
export function endAppDrag() {
  if (started) announce(null);
}

export function dragTypes(event: DragEvent): string[] {
  return Array.from(event.dataTransfer?.types ?? []);
}

export function startThreadDrag(event: DragEvent, threadIds: string[]) {
  startAppDrag(event, { kind: 'threads', ids: threadIds });
}

/**
 * Folders read threads from the drag alone: the list and the sidebar share a
 * page, so its custom types are always visible to them.
 */
export function isThreadDrag(event: DragEvent): boolean {
  return dragTypes(event).includes(THREAD_DRAG_TYPE);
}

export function draggedThreadIds(event: DragEvent): string[] {
  try {
    const drag = JSON.parse(event.dataTransfer?.getData(THREAD_DRAG_TYPE) || 'null') as Partial<AppDrag> | null;
    const ids: unknown = drag?.kind === 'threads' ? drag.ids : null;
    return Array.isArray(ids) ? ids.filter((id): id is string => typeof id === 'string' && id !== '') : [];
  } catch {
    return [];
  }
}
