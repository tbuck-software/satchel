import type { Attachment, Message } from '@/types/mail';
import type { BackendDownloadAttachmentResult } from '@/types/messages';
import {
  backendDownloadAttachment,
  backendOpenAttachmentDefault,
  backendOpenMessageFileInWindow,
  backendToggleAttachmentQuickLook,
} from '@/services/backend';
import { saveAttachmentWithDialog } from '@/utils/saveAttachment';
import { attachmentBytes } from '@/utils/attachmentBytes';
import { backendErrorKey, backendErrorStringParam } from '@/utils/backendErrors';
import { useConfirm } from '@/composables/useConfirm';
import { t } from '@/i18n/runtime';

export interface AttachmentSaveResult {
  directory: string;
  saved: number;
  failed: Attachment[];
}

export interface SaveAttachmentsOptions {
  /** Used when a backend response does not contain a filename. */
  fallbackFilename: string;
}

/**
 * Attachment workflows shared by the chip, preview dialog, and thread
 * toolbar. The UI owns presentation, busy state, and messages; this boundary
 * owns the backend/file-system choreography so those paths cannot drift apart.
 */
export function useAttachmentActions() {
  const { ask } = useConfirm();

  async function downloadAttachment(attachmentId: string): Promise<BackendDownloadAttachmentResult> {
    return backendDownloadAttachment(attachmentId);
  }

  async function saveDownloadedAttachment(
    data: BackendDownloadAttachmentResult,
    fallbackFilename: string,
  ): Promise<void> {
    await saveAttachmentWithDialog(data.filename || fallbackFilename, await attachmentBytes(data));
  }

  async function downloadAndSaveAttachment(
    attachmentId: string,
    fallbackFilename: string,
  ): Promise<BackendDownloadAttachmentResult> {
    const data = await downloadAttachment(attachmentId);
    await saveDownloadedAttachment(data, fallbackFilename);
    return data;
  }

  /**
   * Types the operating system runs are refused by the backend until the user
   * has seen the whole file name and said yes. Both keys carry the name and
   * the extension, so the question can name what is at stake — a chip shows an
   * icon and a truncated name, which is exactly where `Rechnung.pdf.exe`
   * hides.
   */
  const EXECUTABLE_REFUSALS = new Set([
    'backend.errors.attachmentExecutableNotConfirmed',
    'backend.errors.attachmentExecutableDisguisedNotConfirmed',
  ]);

  async function openAttachmentDefault(attachmentId: string): Promise<void> {
    try {
      await backendOpenAttachmentDefault(attachmentId);
    } catch (error) {
      const key = backendErrorKey(error);
      if (!key || !EXECUTABLE_REFUSALS.has(key)) throw error;

      const filename = backendErrorStringParam(error, 'filename') ?? '';
      const extension = backendErrorStringParam(error, 'extension') ?? '';
      const disguised = key.endsWith('DisguisedNotConfirmed');
      const confirmed = await ask({
        title: t('mail.attachment.executableTitle'),
        body: disguised
          ? t('mail.attachment.executableDisguisedBody', { extension })
          : t('mail.attachment.executableBody', { extension }),
        detail: filename,
        confirmLabel: t('mail.attachment.executableConfirm'),
        tone: 'danger',
      });
      if (!confirmed) return;
      await backendOpenAttachmentDefault(attachmentId, true);
    }
  }

  /** Returns false when the current platform has no native Quick Look. */
  async function toggleQuickLook(attachmentId: string): Promise<boolean> {
    return backendToggleAttachmentQuickLook(attachmentId);
  }

  function nestedMessagePath(
    data: BackendDownloadAttachmentResult | null,
    fallbackFilename?: string | null,
    fallbackMimeType?: string | null,
  ): string | null {
    const mimeType = data?.mime_type || fallbackMimeType || '';
    const name = (data?.filename || fallbackFilename || '').toLowerCase();
    const looksLikeMessage =
      mimeType.startsWith('message/rfc822')
      || mimeType === 'application/vnd.ms-outlook'
      || name.endsWith('.eml')
      || name.endsWith('.msg');
    return looksLikeMessage ? (data?.cached_path ?? null) : null;
  }

  async function openMessageFile(path: string): Promise<void> {
    await backendOpenMessageFileInWindow(path);
  }

  function listDownloadableAttachments(messages: readonly Message[]): Attachment[] {
    return messages.flatMap((message) =>
      message.attachments.filter((attachment) => {
        const type = attachment.contentType.toLowerCase();
        const name = attachment.filename.toLowerCase();
        return (
          !attachment.isInline
          && !type.startsWith('text/calendar')
          && type !== 'application/ics'
          && !name.endsWith('.ics')
          && !type.includes('pkcs7-signature')
        );
      }),
    );
  }

  /** Append " (2)", " (3)", … so several parts with the same name survive. */
  function uniqueFilename(name: string, taken: Set<string>, fallbackFilename: string): string {
    const safe = name.replace(/[/\\:]/g, '_').trim() || fallbackFilename;
    if (!taken.has(safe)) {
      taken.add(safe);
      return safe;
    }
    const dot = safe.lastIndexOf('.');
    const stem = dot > 0 ? safe.slice(0, dot) : safe;
    const extension = dot > 0 ? safe.slice(dot) : '';
    for (let index = 2; ; index += 1) {
      const candidate = `${stem} (${index})${extension}`;
      if (!taken.has(candidate)) {
        taken.add(candidate);
        return candidate;
      }
    }
  }

  /**
   * Ask for one directory and continue after an individual download/write
   * failure. A null result means the user cancelled the directory dialog.
   * Dialog errors are deliberately rethrown so the caller can choose its own
   * localized error surface.
   */
  async function saveAttachmentsToDirectory(
    attachments: readonly Attachment[],
    options: SaveAttachmentsOptions,
  ): Promise<AttachmentSaveResult | null> {
    const dialog = await import('@tauri-apps/plugin-dialog');
    const fs = await import('@tauri-apps/plugin-fs');
    const directory = await dialog.open({ directory: true, multiple: false });
    if (typeof directory !== 'string') return null;

    const taken = new Set<string>();
    let saved = 0;
    const failed: Attachment[] = [];

    for (const attachment of attachments) {
      try {
        const data = await downloadAttachment(attachment.id);
        const name = uniqueFilename(
          data.filename || attachment.filename,
          taken,
          options.fallbackFilename,
        );
        const bytes = await attachmentBytes(data);
        await fs.writeFile(`${directory}/${name}`, bytes);
        saved += 1;
      } catch {
        // One unreadable part should not cost the user the rest of the batch.
        failed.push(attachment);
      }
    }

    return { directory, saved, failed };
  }

  return {
    downloadAttachment,
    saveDownloadedAttachment,
    downloadAndSaveAttachment,
    openAttachmentDefault,
    toggleQuickLook,
    nestedMessagePath,
    openMessageFile,
    listDownloadableAttachments,
    saveAttachmentsToDirectory,
  };
}
