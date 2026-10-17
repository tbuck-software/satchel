import type { Attachment } from '@/types/mail';
import { isSmimeSignatureAttachment } from './messageAttachments';

/**
 * Inline images below this size are almost always signature logos or tracking
 * decor; real inline content (screenshots, photos) is bigger.
 */
const SIGNATURE_INLINE_MAX_BYTES = 32_000;

/**
 * Whether an attachment is worth showing in an attachment list.
 *
 * Filters two kinds of noise that mail parsers store as attachments:
 * - nameless `text/plain` / `text/html` parts, which are the mail body's own
 *   alternatives, not files ('attachment' is the store's fallback name);
 * - small inline (cid) images, which are signature logos rather than content.
 *   Large inline images (embedded screenshots, photos) stay.
 */
export function isListWorthyAttachment(attachment: Attachment): boolean {
  const hasRealName = Boolean(attachment.filename) && attachment.filename !== 'attachment';
  const type = (attachment.contentType || '').toLowerCase();
  if (isSmimeSignatureAttachment(attachment)) return false;
  if (!hasRealName && (type.startsWith('text/plain') || type.startsWith('text/html'))) {
    return false;
  }
  if (attachment.isInline || attachment.contentId) {
    return attachment.size >= SIGNATURE_INLINE_MAX_BYTES;
  }
  return true;
}
