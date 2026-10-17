import { backendSupportsNativeCommands } from '@/services/backend/native';
import { backendOpenExternalUrl } from '@/services/backend/system';
import { BackendError } from '@/utils/backendErrors';

/**
 * Whether external URLs should be handed to the native shell instead of the
 * browser. Kept next to the opener so browser-only entry points (including
 * the message-file viewer) do not need to know about Tauri.
 */
export function supportsNativeExternalUrls(): boolean {
  return backendSupportsNativeCommands();
}

/**
 * Open an external URL through the native shell or the browser fallback.
 *
 * The browser opener is deliberately checked: a blocked popup is a failed
 * action, not a successful no-op. Callers that show activation errors rely on
 * this rejection to keep their failure path visible.
 */
export async function openExternalUrl(url: string): Promise<void> {
  if (supportsNativeExternalUrls()) {
    await backendOpenExternalUrl(url);
    return;
  }

  if (typeof window === 'undefined') {
    throw new BackendError('backend.errors.browserWindowUnavailable');
  }

  const opened = window.open(url, '_blank', 'noopener,noreferrer');
  if (!opened) {
    throw new BackendError('backend.errors.browserWindowBlocked');
  }
  opened.opener = null;
}
