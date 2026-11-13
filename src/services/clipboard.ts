import { backendSupportsNativeCommands } from '@/services/backend/native';

/**
 * The clipboard as plain text, or null when it holds none or cannot be read.
 *
 * Natively through the clipboard plugin: WKWebView answers
 * `navigator.clipboard.readText()` with a "Paste" popup the user must click.
 */
export async function readClipboardText(): Promise<string | null> {
  try {
    if (backendSupportsNativeCommands()) {
      const { readText } = await import('@tauri-apps/plugin-clipboard-manager');
      return await readText();
    }
    return await navigator.clipboard.readText();
  } catch {
    return null;
  }
}
