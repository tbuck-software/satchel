import type { BackendOutgoingAttachment } from '@/types/composer';

/** Ceiling for one attachment. */
export const MAX_ATTACHMENT_BYTES = 25_000_000;

/**
 * Total budget across all attachments. Most providers reject well before this
 * (Gmail ~25 MB, Graph ~35 MB) and base64 adds ~33 % on the wire, so refusing
 * here is far kinder than a failed send after a long upload.
 */
export const MAX_TOTAL_ATTACHMENT_BYTES = 20_000_000;

/** Approximate original byte size of an already-encoded attachment. */
export function encodedAttachmentSize(attachment: BackendOutgoingAttachment): number {
  return Math.floor((attachment.content_base64.length * 3) / 4);
}

export function attachmentsSize(attachments: BackendOutgoingAttachment[]): number {
  return attachments.reduce((sum, attachment) => sum + encodedAttachmentSize(attachment), 0);
}
