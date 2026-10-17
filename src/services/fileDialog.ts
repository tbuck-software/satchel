/**
 * The native file dialogs, behind one door.
 *
 * Components must not import `@tauri-apps` themselves — the boundary scan says
 * so, and the reason is here: the plugin is loaded lazily, `open()` returns
 * either a path or an array depending on flags nobody wants to re-read at each
 * call site, and there is no dialog at all outside the desktop app.
 */

export interface FileFilter {
  name: string;
  extensions: string[];
}

async function dialog() {
  return import('@tauri-apps/plugin-dialog');
}

/** A directory, or null when the user cancelled. */
export async function pickDirectory(defaultPath?: string): Promise<string | null> {
  const selected = await (await dialog()).open({
    directory: true,
    multiple: false,
    defaultPath: defaultPath || undefined,
  });
  // `multiple: false` never yields an array; narrowing it here keeps the
  // callers from repeating the check.
  return typeof selected === 'string' ? selected : null;
}

/** One existing file, or null when the user cancelled. */
export async function pickFile(filters: FileFilter[]): Promise<string | null> {
  const selected = await (await dialog()).open({ multiple: false, filters });
  return typeof selected === 'string' ? selected : null;
}
