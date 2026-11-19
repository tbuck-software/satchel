import type { Message } from '@/types/mail';
import { cidCandidates, cleanCid } from './emailInlineImages';

type MessageAttachment = Message['attachments'][number];

export function isSmimeSignatureAttachment(attachment: MessageAttachment): boolean {
  const contentType = attachment.contentType.toLowerCase();
  const filename = attachment.filename.toLowerCase();
  return contentType.includes('pkcs7-signature')
    || contentType.includes('x-pkcs7-signature')
    || filename.endsWith('.p7s');
}

/** Signature files are MIME machinery. Opening them as documents is neither
 * useful nor reliable; their presence is represented by the security status.
 * Calendar invites show as the invite card. */
export function isSystemAttachment(attachment: MessageAttachment): boolean {
  const contentType = attachment.contentType.toLowerCase();
  const filename = attachment.filename.toLowerCase();
  return (
    contentType.startsWith('text/calendar')
    || contentType.startsWith('application/ics')
    || filename.endsWith('.ics')
    || contentType.startsWith('application/pgp-signature')
    || isSmimeSignatureAttachment(attachment)
  );
}

/**
 * Whether `html` (lowercased) shows this part at a `cid:` reference. An inline
 * part nothing references (GroupWise sends screenshots that way) is shown
 * nowhere else and has to stay in the attachment list.
 */
function isShownInBody(attachment: MessageAttachment, html: string): boolean {
  return html.includes('cid:') && cidCandidates(attachment)
    .some((candidate) => html.includes(`cid:${cleanCid(candidate).toLowerCase()}`));
}

/**
 * The parts a reader can open or save. The backend sets the list's paperclip
 * by the same rules (`db::attachment_flag`); change both together.
 */
export function listedAttachments(
  message: Pick<Message, 'attachments' | 'bodyHtml' | 'bodyLoaded'>,
): MessageAttachment[] {
  // Until the body is there, an inline part with a Content-ID is taken to be one it shows.
  const bodyPending = message.bodyLoaded === false;
  const html = (message.bodyHtml ?? '').toLowerCase();
  return message.attachments.filter((attachment) =>
    !isSystemAttachment(attachment)
    && !(bodyPending && attachment.isInline && attachment.contentId)
    && !isShownInBody(attachment, html));
}
