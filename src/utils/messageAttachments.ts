import type { Message } from '@/types/mail';

type MessageAttachment = Message['attachments'][number];

export function isSmimeSignatureAttachment(attachment: MessageAttachment): boolean {
  const contentType = attachment.contentType.toLowerCase();
  const filename = attachment.filename.toLowerCase();
  return contentType.includes('pkcs7-signature')
    || contentType.includes('x-pkcs7-signature')
    || filename.endsWith('.p7s');
}

/** Signature files are MIME machinery. Opening them as documents is neither
 * useful nor reliable; their presence is represented by the security status. */
export function isSystemAttachment(attachment: MessageAttachment): boolean {
  const contentType = attachment.contentType.toLowerCase();
  const filename = attachment.filename.toLowerCase();
  return (
    attachment.isInline
    || contentType.startsWith('text/calendar')
    || contentType === 'application/ics'
    || filename.endsWith('.ics')
    || isSmimeSignatureAttachment(attachment)
  );
}
