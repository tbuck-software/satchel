import type { Attachment } from '@/types/mail';

/**
 * Resolve `cid:` references in mail HTML against the message's own attachments.
 *
 * Inline images travel as separate MIME parts, referenced from the HTML by
 * Content-ID. Nothing fetches those references for us: the mail document is
 * sandboxed and its CSP has no origin to resolve `cid:` against, so an
 * unresolved reference renders as a broken image. Rewriting them to the
 * attachment's data URL keeps the substitution local — no request leaves the
 * client, which is the point.
 *
 * Matching is deliberately generous. Content-IDs arrive bare, in angle
 * brackets, or percent-encoded, and some senders reference the filename
 * instead, so every plausible spelling is tried before giving up.
 */

/**
 * Shown where an inline image was referenced but not delivered — a partially
 * downloaded message, or a sender who forgot the attachment. A visible
 * placeholder beats an invisible broken image, and being a data URL it costs
 * no request.
 */
export function missingInlineImageSource(label = 'Image unavailable'): string {
  const text = label.replace(/[<>&"']/g, '');
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="240" height="80" viewBox="0 0 240 80">'
    + '<rect x="1" y="1" width="238" height="78" rx="8" fill="none" stroke="#7c828c" stroke-dasharray="4 4"/>'
    + `<text x="120" y="44" font-family="Arial, sans-serif" font-size="13" fill="#7c828c" text-anchor="middle">${text}</text></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export function cleanCid(value: string): string {
  const stripped = value
    .trim()
    .replace(/^cid:/i, '')
    .replace(/^<|>$/g, '')
    .trim();
  try {
    return decodeURIComponent(stripped);
  } catch {
    return stripped;
  }
}

/** Every spelling under which this attachment might be referenced. */
export function cidCandidates(attachment: Attachment): string[] {
  const candidates = new Set<string>();
  if (attachment.contentId) {
    const cleaned = cleanCid(attachment.contentId);
    candidates.add(cleaned);
    candidates.add(`<${cleaned}>`);
  }
  if (attachment.filename) {
    candidates.add(attachment.filename);
  }
  return Array.from(candidates).filter(Boolean);
}

const mimeToken = "[a-z0-9!#$%&'*+.^_`|~-]+";
const imageMimePattern = new RegExp(`^image/${mimeToken}$`, 'i');
const imageDataPattern = new RegExp(`^data:image/${mimeToken};base64,[a-z0-9+/=\\s]*$`, 'i');

export function isImageMimeType(value: string): boolean {
  return imageMimePattern.test(value);
}

/** Validate independently of provider metadata, including old cached rows. */
export function inlineImageSource(attachment: Attachment): string | null {
  return isImageMimeType(attachment.contentType)
    && attachment.dataUrl && imageDataPattern.test(attachment.dataUrl)
    ? attachment.dataUrl : null;
}

export function rewriteInlineAttachmentSources(
  html: string,
  attachments: Attachment[],
  options: { attachmentsResolved: boolean; missingSource: string; keepPendingAttachments?: boolean },
): { html: string; pending: number } {
  if (!html || !/cid:/i.test(html)) return { html, pending: 0 };
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const known = new Map<string, Attachment>();
  for (const attachment of attachments) {
    for (const candidate of cidCandidates(attachment)) {
      const key = cleanCid(candidate).toLowerCase();
      if (!known.has(key)) known.set(key, attachment);
    }
  }
  let pending = 0;
  for (const image of doc.querySelectorAll<HTMLImageElement>('img[src]')) {
    const reference = image.getAttribute('src') ?? '';
    if (!/^cid:/i.test(reference)) continue;
    const attachment = known.get(cleanCid(reference).toLowerCase());
    const source = attachment ? inlineImageSource(attachment) : null;
    if (source) image.src = source;
    else if (!options.attachmentsResolved || (attachment && options.keepPendingAttachments !== false)) pending += 1;
    else image.src = options.missingSource;
  }

  // Other image references are assigned to nodes too. Only validated base64
  // URLs can enter raw stylesheet text, so serialization cannot close a style.
  const sourceForReference = (reference: string) => {
    const attachment = known.get(cleanCid(reference).toLowerCase());
    return attachment ? inlineImageSource(attachment) ?? reference : reference;
  };
  const replaceReferences = (value: string) => value.replace(
    /cid:<[^>]*>|cid:[^"'>),]+/gi, sourceForReference,
  );
  for (const element of doc.querySelectorAll('[style], img[srcset], [background]')) {
    for (const attribute of ['style', 'srcset', 'background']) {
      const value = element.getAttribute(attribute);
      if (value) element.setAttribute(attribute, attribute === 'srcset'
        ? value.replace(/cid:<[^>]*>|cid:[^\s,]+/gi, sourceForReference)
        : replaceReferences(value));
    }
  }
  for (const style of doc.querySelectorAll('style')) {
    style.textContent = replaceReferences(style.textContent ?? '');
  }
  return { html: doc.documentElement.outerHTML, pending };
}

export function applyInlineAttachmentSources(
  html: string,
  attachments: Attachment[],
  missingSource = missingInlineImageSource(),
): string {
  return rewriteInlineAttachmentSources(html, attachments, {
    attachmentsResolved: true, missingSource, keepPendingAttachments: false,
  }).html;
}

/** Content IDs the body references from an `<img>`, normalized for lookup. */
export function referencedInlineCids(html: string): Set<string> {
  const referenced = new Set<string>();
  if (!html || !/cid:/i.test(html)) return referenced;
  const doc = new DOMParser().parseFromString(html, 'text/html');
  for (const image of doc.querySelectorAll('img[src]')) {
    const source = image.getAttribute('src') ?? '';
    if (/^cid:/i.test(source)) referenced.add(cleanCid(source).toLowerCase());
  }
  return referenced;
}

/**
 * Referenced images still waiting for their bytes. A reference with no
 * attachment row at all only counts while the listing is outstanding; after
 * that it is gone for good and renders as the placeholder.
 */
export function pendingInlineImageCount(
  referencedCids: ReadonlySet<string>,
  attachments: Attachment[],
  attachmentsResolved: boolean,
): number {
  const known = new Map<string, Attachment>();
  for (const attachment of attachments) {
    for (const candidate of cidCandidates(attachment)) {
      const key = cleanCid(candidate).toLowerCase();
      if (!known.has(key)) known.set(key, attachment);
    }
  }
  let pending = 0;
  for (const cid of referencedCids) {
    const attachment = known.get(cid);
    if (attachment && inlineImageSource(attachment)) continue;
    if (attachment || !attachmentsResolved) pending += 1;
  }
  return pending;
}
