import { backendSupportsNativeCommands } from '@/services/backend';

export type TextExportResult = string | true | false;

/**
 * Save text through the native file dialog when available and through a
 * browser download everywhere else.
 *
 * A native result is the selected path, a browser result is `true`, and a
 * cancelled dialog is `false`. File-system and browser errors deliberately
 * propagate so the caller can choose its own localized error surface.
 */
export async function saveTextExport(
  filename: string,
  content: string,
  mimeType: string,
  extension: string,
): Promise<TextExportResult> {
  if (backendSupportsNativeCommands()) {
    const dialog = await import('@tauri-apps/plugin-dialog');
    const path = await dialog.save({
      defaultPath: filename,
      filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
    });
    if (!path) return false;
    const fs = await import('@tauri-apps/plugin-fs');
    await fs.writeTextFile(path, content);
    return path;
  }

  const url = URL.createObjectURL(new Blob([content], { type: `${mimeType};charset=utf-8` }));
  try {
    const link = document.createElement('a');
    link.href = url;
    link.download = filename;
    link.click();
  } finally {
    URL.revokeObjectURL(url);
  }
  return true;
}
