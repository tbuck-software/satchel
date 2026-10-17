import { useToast, type Toast } from '@/composables/useToast';
import { t } from '@/i18n/runtime';
import { backendShowInFolder } from '@/services/backend/system';

/**
 * Toast action for "saved" confirmations: reveals the file in the system file
 * manager (directories open directly). Failures surface as their own toast —
 * the original toast is already dismissed once the action runs.
 */
export function showInFolderAction(
  path: string,
  label = t('mail.attachment.showInFolder'),
): NonNullable<Toast['action']> {
  return {
    label,
    handler: () => {
      void backendShowInFolder(path).catch((err) => {
        useToast().error(t('mail.attachment.folderFailed', { error: String(err) }));
      });
    },
  };
}
