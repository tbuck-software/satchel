/**
 * The bits of the native window a component is allowed to touch.
 *
 * Only dragging so far. It lives here rather than in a component because the
 * boundary scan holds `components/` and `views/` clear of `@tauri-apps`, and
 * because the call has to be a no-op in the browser build instead of a throw.
 */
export async function startWindowDrag(): Promise<void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().startDragging();
}
