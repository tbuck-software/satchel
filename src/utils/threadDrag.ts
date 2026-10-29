/**
 * Thread rows dragged out of the list.
 *
 * Folders take them as a move, the composer as a copy (each thread becomes an
 * `.eml` attachment), so the drag allows both and each target names its own
 * effect.
 */
export const THREAD_DRAG_TYPE = 'application/x-satchel-thread-ids';

export function startThreadDrag(event: DragEvent, threadIds: string[]) {
  if (!event.dataTransfer) return;
  event.dataTransfer.setData(THREAD_DRAG_TYPE, threadIds.join(','));
  event.dataTransfer.effectAllowed = 'copyMove';
}

/**
 * Whether a drag carries threads. Only the type list is readable before the
 * drop; `getData` stays empty during `dragover`.
 */
export function isThreadDrag(event: DragEvent): boolean {
  return Array.from(event.dataTransfer?.types ?? []).includes(THREAD_DRAG_TYPE);
}

export function draggedThreadIds(event: DragEvent): string[] {
  const data = event.dataTransfer?.getData(THREAD_DRAG_TYPE) ?? '';
  return data.split(',').filter(Boolean);
}
