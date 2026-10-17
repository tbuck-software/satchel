import { useToast } from '@/composables/useToast';
import { t } from '@/i18n/runtime';
import { showInFolderAction } from '@/utils/showInFolderAction';

/**
 * Native save dialog → write → confirmation toast with a reveal action.
 * Errors surface as a toast here rather than being rethrown: the fs ACL
 * rejects a write before any file appears, and the dialog has already closed
 * by then — a swallowed failure would read as "the button does nothing".
 */
export async function saveAttachmentWithDialog(
  filename: string,
  payload: string | Uint8Array,
): Promise<void> {
  const toast = useToast();
  try {
    const dialog = await import('@tauri-apps/plugin-dialog');
    const path = await dialog.save({ defaultPath: filename });
    if (!path) return;
    const fs = await import('@tauri-apps/plugin-fs');
    const bytes = typeof payload === 'string'
      ? Uint8Array.from(atob(payload), (char) => char.charCodeAt(0))
      : payload;
    await fs.writeFile(path, bytes);
    toast.success(t('mail.attachment.saved', { filename }), { action: showInFolderAction(path) });
  } catch (err) {
    toast.error(t('mail.attachment.saveFailed', { error: String(err) }));
  }
}
