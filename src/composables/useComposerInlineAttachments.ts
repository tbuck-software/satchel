import { nextTick, type Ref } from 'vue';
import type { BackendOutgoingAttachment, InlineImagePayload } from '@/types/composer';
import { bytesToBase64 } from '@/utils/base64';
import {
  hydrateCidReferences,
  inlineOnly,
  serializeCidReferences,
} from '@/utils/composerInlineCids';

/** The small part of the editor surface needed by attachment workflows. */
export interface ComposerInlineAttachmentEditor {
  insertAttachmentImage(image: BackendOutgoingAttachment): void;
  removeAttachmentImage(image: BackendOutgoingAttachment): void;
}

/**
 * Owns the translation and attachment bookkeeping for images in the composer.
 *
 * The composer keeps the attachment list, while this composable owns the
 * rules that make an image an inline CID attachment or a regular attachment.
 * The editor is supplied as an accessor so delayed editor calls always see
 * the current component instance without coupling this composable to Vue's
 * component type.
 */
export function useComposerInlineAttachments(
  attachments: Ref<BackendOutgoingAttachment[]>,
  getEditor: () => ComposerInlineAttachmentEditor | null,
) {
  let pasteQueue: Promise<void> = Promise.resolve();

  function hydrateInlineImages(
    html: string | null,
    sourceAttachments: BackendOutgoingAttachment[],
  ) {
    return hydrateCidReferences(html, inlineOnly(sourceAttachments));
  }

  function serializeInlineImages(html: string | null) {
    return serializeCidReferences(html, inlineOnly(attachments.value));
  }

  function newInlineContentId() {
    return `satchel-${crypto.randomUUID()}@inline`;
  }

  function onInlineImageAdded(image: InlineImagePayload) {
    const existing = attachments.value.findIndex(
      (attachment) => attachment.content_base64 === image.contentBase64,
    );
    const inlineAttachment: BackendOutgoingAttachment = {
      filename: image.filename,
      mime_type: image.mimeType,
      content_base64: image.contentBase64,
      content_id:
        existing >= 0 && attachments.value[existing]
          ? attachments.value[existing]!.content_id || newInlineContentId()
          : newInlineContentId(),
      inline: true,
    };
    if (existing >= 0) {
      const next = [...attachments.value];
      next[existing] = inlineAttachment;
      attachments.value = next;
    } else {
      attachments.value = [...attachments.value, inlineAttachment];
    }
  }

  function onImageToAttachment(image: InlineImagePayload) {
    const existing = attachments.value.findIndex(
      (attachment) => attachment.content_base64 === image.contentBase64,
    );
    const regularAttachment: BackendOutgoingAttachment = {
      filename: image.filename,
      mime_type: image.mimeType,
      content_base64: image.contentBase64,
      content_id: null,
      inline: false,
    };
    if (existing >= 0) {
      const next = [...attachments.value];
      next[existing] = regularAttachment;
      attachments.value = next;
    } else {
      attachments.value = [...attachments.value, regularAttachment];
    }
  }

  function onAttachmentInsertedInline(attachment: BackendOutgoingAttachment) {
    nextTick(() => getEditor()?.insertAttachmentImage(attachment));
  }

  function onInlineAttachmentRemoved(attachment: BackendOutgoingAttachment) {
    nextTick(() => getEditor()?.removeAttachmentImage(attachment));
  }

  function addPastedFiles(files: File[]): Promise<void> {
    const operation = pasteQueue.then(async () => {
      const additions: BackendOutgoingAttachment[] = [];
      for (const file of files) {
        additions.push({
          filename: file.name,
          mime_type: file.type || 'application/octet-stream',
          content_base64: bytesToBase64(new Uint8Array(await file.arrayBuffer())),
          inline: false,
        });
      }
      attachments.value = [...attachments.value, ...additions];
    });
    pasteQueue = operation.catch(() => undefined);
    return operation;
  }

  return {
    hydrateInlineImages,
    serializeInlineImages,
    onInlineImageAdded,
    onImageToAttachment,
    onAttachmentInsertedInline,
    onInlineAttachmentRemoved,
    addPastedFiles,
  };
}
