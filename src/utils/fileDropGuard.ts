/**
 * Files dropped where nothing takes them. Without this the webview opens the
 * file in place of the app (then only the navigation guard would stand in the
 * way) and the cursor promises a drop that does nothing.
 *
 * Bubble phase on the window, registered after every other window-level
 * handler: whatever accepted the drag below it (the attachment zone, the
 * composer, the `.ics` import) has called `preventDefault` by then, and only
 * an unclaimed file drag is answered with "not allowed". In the capture phase
 * the guard could not know whether a zone further down will take it.
 */
export function installFileDropGuard(target: Window = window): () => void {
  const carriesFiles = (event: DragEvent) => Array.from(event.dataTransfer?.types ?? []).includes('Files');
  const onDragOver = (event: DragEvent) => {
    if (event.defaultPrevented || !carriesFiles(event)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'none';
  };
  const onDrop = (event: DragEvent) => {
    if (carriesFiles(event)) event.preventDefault();
  };
  target.addEventListener('dragover', onDragOver);
  target.addEventListener('drop', onDrop);
  return () => {
    target.removeEventListener('dragover', onDragOver);
    target.removeEventListener('drop', onDrop);
  };
}
