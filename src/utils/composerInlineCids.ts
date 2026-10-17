/**
 * Translate outgoing HTML between the two forms an inline image takes.
 *
 * The editor can only show what the browser can load, so while composing, an
 * inline image is a data URL in the markup. On the wire it has to be a `cid:`
 * reference to a separate MIME part — inlining megabytes of base64 into the
 * body would bloat the message and break clients that expect the multipart
 * form. So the body is hydrated on load and serialised again before sending.
 *
 * The round trip has to be exact. A data URL left in the body is sent as-is
 * and shows up as a broken image for the recipient; a `cid:` reference left in
 * the editor shows as broken for the sender.
 *
 * Signature images and pasted inline images need the same translation and
 * differ only in which attachments take part, so the caller supplies the list.
 */

/** The attachment fields this translation needs, structurally. */
export interface CidAttachment {
  mime_type: string;
  content_base64: string;
  content_id?: string | null;
}

export function attachmentDataUrl(attachment: CidAttachment): string {
  return `data:${attachment.mime_type};base64,${attachment.content_base64}`;
}

/** `cid:` references to data URLs, for display in the editor. */
export function hydrateCidReferences<T extends CidAttachment>(
  html: string | null,
  attachments: readonly T[],
): string | null {
  if (!html) return html;
  let hydrated = html;
  for (const attachment of attachments) {
    if (!attachment.content_id) continue;
    hydrated = hydrated
      .split(`cid:${attachment.content_id}`)
      .join(attachmentDataUrl(attachment));
  }
  return hydrated;
}

/** Data URLs back to `cid:` references, for the wire. */
export function serializeCidReferences<T extends CidAttachment>(
  html: string | null,
  attachments: readonly T[],
): string | null {
  if (!html) return html;
  let serialized = html;
  for (const attachment of attachments) {
    if (!attachment.content_id) continue;
    serialized = serialized
      .split(attachmentDataUrl(attachment))
      .join(`cid:${attachment.content_id}`);
  }
  return serialized;
}

/** Only attachments marked inline take part in the body translation. */
export function inlineOnly<T extends CidAttachment & { inline?: boolean }>(
  attachments: readonly T[],
): T[] {
  return attachments.filter((attachment) => Boolean(attachment.inline));
}
