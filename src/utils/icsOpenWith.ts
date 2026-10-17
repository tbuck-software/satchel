/**
 * The cold-start half of "Open with Satchel".
 *
 * A running app hears `ics:open-with`. A launch from the Finder emits it
 * before any webview exists, so the backend buffers the path instead; this
 * hands those buffered paths to the same importer. The caller has to register
 * its listener first — the buffer is emptied by reading it, so anything that
 * arrives afterwards belongs to the listener alone.
 */
export async function drainPendingIcsPaths(
  takePending: () => Promise<string[]>,
  importPath: (path: string) => Promise<void>,
): Promise<void> {
  let paths: string[];
  try {
    paths = await takePending();
  } catch (error) {
    // An older shell has no such command; a launch simply carried nothing.
    console.warn('pending open-with paths unavailable', error);
    return;
  }
  for (const path of paths) {
    await importPath(path);
  }
}
