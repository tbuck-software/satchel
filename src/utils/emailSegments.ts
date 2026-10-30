import type { Attachment, Message } from '@/types/mail';
/** Decryption provenance comes only from a backend result, never mail text. */
export interface RenderSegment {
  kind: 'text' | 'pgp_decrypted' | 'pgp_encrypted' | 'smime_decrypted';
  content: string;
}

const ENCRYPTED_BEGIN = '-----BEGIN PGP MESSAGE-----';
const ENCRYPTED_END = '-----END PGP MESSAGE-----';

function stripQuotePrefix(line: string): string {
  let current = line.trimStart();
  while (current.startsWith('>')) {
    current = current.slice(1).trimStart();
  }
  return current;
}

/**
 * Find PGP armor in plain text.
 *
 * Quote markers are stripped per line before matching, because armor that has
 * been through a reply arrives as `> -----BEGIN PGP MESSAGE-----`. The stripped
 * form is what gets collected — the block has to be byte-exact to decrypt.
 * Unterminated armor falls back to text so nothing is swallowed.
 */
export function splitEncryptedSegments(input: string): RenderSegment[] {
  const normalized = input.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
  const lines = normalized.split('\n');
  if (!normalized.includes(ENCRYPTED_BEGIN)) {
    return [{ kind: 'text', content: normalized }];
  }

  const output: RenderSegment[] = [];
  let textBuffer = '';
  let encryptedBuffer = '';
  let collecting = false;

  for (const rawLine of lines) {
    const prepared = stripQuotePrefix(rawLine);
    if (!collecting) {
      const beginIndex = prepared.indexOf(ENCRYPTED_BEGIN);
      if (beginIndex >= 0) {
        if (textBuffer.trim()) {
          output.push({ kind: 'text', content: textBuffer.trimEnd() });
        }
        textBuffer = '';
        collecting = true;
        encryptedBuffer = `${prepared.slice(beginIndex).trimEnd()}\n`;
        if (prepared.includes(ENCRYPTED_END)) {
          output.push({ kind: 'pgp_encrypted', content: encryptedBuffer.trimEnd() });
          encryptedBuffer = '';
          collecting = false;
        }
      } else {
        textBuffer += `${rawLine}\n`;
      }
      continue;
    }

    encryptedBuffer += `${prepared.trimEnd()}\n`;
    if (prepared.includes(ENCRYPTED_END)) {
      output.push({ kind: 'pgp_encrypted', content: encryptedBuffer.trimEnd() });
      encryptedBuffer = '';
      collecting = false;
    }
  }

  if (collecting && encryptedBuffer.trim()) {
    textBuffer += encryptedBuffer;
  }
  if (textBuffer.trim()) {
    output.push({ kind: 'text', content: textBuffer.trimEnd() });
  }

  return output.length > 0 ? output : [{ kind: 'text', content: normalized }];
}

/** Reject stale or mismatched provenance before any display normalization. */
export function matchingBodySegments(text: string, segments?: RenderSegment[] | null): RenderSegment[] | undefined {
  return segments?.length && segments.map((segment) => segment.content).join('') === text
    ? segments : undefined;
}

export function splitRenderSegments(text: string, provenance?: RenderSegment[] | null): RenderSegment[] {
  const segments = matchingBodySegments(text, provenance) ?? [{ kind: 'text' as const, content: text }];
  return segments.flatMap((segment) => segment.kind === 'text'
    ? splitEncryptedSegments(segment.content) : [segment])
    .filter((segment) => segment.content.trim().length > 0);
}


/** Exact HTML and published attachment inputs belong to the same decoded text.
 * This snapshot is transient IPC data, never parsed from sender-controlled MIME.
 * Attachments match by position: a `cid:` reference resolves to the first
 * attachment that carries it, so the order decides which image is shown.
 * PR54 must additionally validate the saved ContentReference/generation.
 */
export function matchingBodyProvenance(
  text: string, html: string | undefined, segments: RenderSegment[] | undefined,
  provenance: Message['bodyProvenance'], attachments: Attachment[],
): RenderSegment[] | undefined {
  if (!provenance || (provenance.html ?? undefined) !== html) return undefined;
  if (!segments || segments.length !== provenance.segments.length
    || !segments.every((segment, index) => (
      segment.kind === provenance.segments[index]?.kind
      && segment.content === provenance.segments[index]?.content
    ))) return undefined;
  if (attachments.length !== provenance.attachments.length) return undefined;
  if (!provenance.attachments.every((expected, index) => {
    const actual = attachments[index]!;
    return actual.id === expected.id && actual.filename === expected.filename
      && actual.contentType === expected.content_type && actual.size === expected.size
      && (actual.contentId ?? null) === expected.content_id
      && Boolean(actual.isInline) === expected.is_inline
      && (actual.cachedPath ?? null) === expected.cached_path
      && (!actual.dataUrl || actual.dataUrl === expected.data_url);
  })) return undefined;
  return matchingBodySegments(text, segments);
}
