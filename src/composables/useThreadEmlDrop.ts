import { ref, type Ref } from 'vue';
import { t, tc } from '@/i18n/runtime';
import { backendMessageEmlAttachments, backendThreadEmlAttachments } from '@/services/backend/composer';
import type { BackendOutgoingAttachment } from '@/types/composer';
import {
  attachmentsSize,
  encodedAttachmentSize,
  MAX_ATTACHMENT_BYTES,
  MAX_TOTAL_ATTACHMENT_BYTES,
} from '@/utils/attachmentLimits';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { formatBytes } from '@/utils/formatBytes';
import { appDragKind, finishAppDrag, readAppDrag } from '@/utils/appDragTarget';

export interface ThreadEmlDropFeedback {
  /** The originals are on their way; shown until `done` or `error`. */
  loading(text: string): void;
  done(): void;
  error(text: string): void;
}

/** What a drop or a command hands the composer to attach. */
export type EmlSource = { kind: 'threads' | 'messages'; ids: string[] };

/**
 * Lets the composer take mail dragged onto it: a thread out of a list arrives
 * as its newest message, a message (a search hit, one message of a thread) as
 * itself, each attached as the original `.eml` file.
 *
 * The handlers belong on the composer's root in the capture phase. They stop
 * mail drags there, over the recipient fields too, so the editor does not
 * paste the drag and the file drop zone does not light up for something that
 * is not a file; every other drag passes through untouched.
 */
export function useThreadEmlDrop(
  attachments: Ref<BackendOutgoingAttachment[]>,
  feedback: ThreadEmlDropFeedback,
  disabled: () => boolean = () => false,
) {
  const dropActive = ref(false);
  const attaching = ref(false);
  // dragenter/dragleave fire for every child crossed; the depth says when the
  // pointer really left the composer.
  let depth = 0;

  function claim(event: DragEvent): boolean {
    const kind = appDragKind(event);
    if (kind !== 'threads' && kind !== 'messages') return false;
    event.preventDefault();
    event.stopPropagation();
    return true;
  }

  function onDragEnter(event: DragEvent) {
    if (!claim(event)) return;
    depth += 1;
    dropActive.value = !disabled();
  }

  function onDragOver(event: DragEvent) {
    if (!claim(event)) return;
    if (event.dataTransfer) event.dataTransfer.dropEffect = disabled() ? 'none' : 'copy';
  }

  function onDragLeave(event: DragEvent) {
    if (!claim(event)) return;
    depth = Math.max(0, depth - 1);
    if (depth === 0) dropActive.value = false;
  }

  function onDrop(event: DragEvent) {
    if (!claim(event)) return;
    depth = 0;
    dropActive.value = false;
    const drag = readAppDrag(event);
    finishAppDrag();
    if (!drag || drag.kind === 'recipients') return;
    void attach(drag);
  }

  /** Attaches the originals; the keyboard path ("Attach to draft") lands here too. */
  async function attach(source: EmlSource) {
    if (source.ids.length === 0 || disabled() || attaching.value) return;
    attaching.value = true;
    feedback.loading(t('compose.emlDrop.loading'));
    try {
      const fetched = source.kind === 'messages'
        ? await backendMessageEmlAttachments(source.ids)
        : await backendThreadEmlAttachments(source.ids);
      // Read after the await: attachments added or removed meanwhile stay.
      let used = attachmentsSize(attachments.value);
      const added: BackendOutgoingAttachment[] = [];
      const refused: string[] = [];
      for (const attachment of fetched) {
        const size = encodedAttachmentSize(attachment);
        if (size > MAX_ATTACHMENT_BYTES) {
          refused.push(t('mail.attachment.tooLarge', {
            name: attachment.filename,
            limit: formatBytes(MAX_ATTACHMENT_BYTES),
          }));
          continue;
        }
        if (used + size > MAX_TOTAL_ATTACHMENT_BYTES) {
          refused.push(t('mail.attachment.totalExceeded', {
            name: attachment.filename,
            limit: formatBytes(MAX_TOTAL_ATTACHMENT_BYTES),
            used: formatBytes(used),
          }));
          continue;
        }
        added.push(attachment);
        used += size;
      }
      if (added.length) attachments.value = [...attachments.value, ...added];
      const missing = new Set(source.ids).size - fetched.length;
      if (missing > 0) refused.unshift(tc('compose.emlDrop.unavailable', missing));
      if (refused.length) feedback.error(refused.join(' '));
      else feedback.done();
    } catch (error) {
      feedback.error(backendErrorLabel(error));
    } finally {
      attaching.value = false;
    }
  }

  return {
    dropActive,
    attaching,
    attach,
    onDragEnter,
    onDragOver,
    onDragLeave,
    onDrop,
  };
}
