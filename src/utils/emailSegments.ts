/**
 * Split a message body into plain, decrypted and still-encrypted stretches.
 *
 * A single mail can mix them: a plain-text cover note above an encrypted block,
 * a decrypted section quoted inside a reply, or several PGP blocks in sequence.
 * The reader has to distinguish them, because "this text was cryptographically
 * verified" and "this text was not" must never look alike.
 *
 * Decrypted regions arrive from the backend fenced in sentinel markers — the
 * backend has already verified them, and the sentinels carry that fact across
 * the IPC boundary. Still-encrypted regions are recognised by their PGP armor,
 * which is what the sender actually transmitted.
 */

const DECRYPTED_START = '[[SATCHEL_PGP_DECRYPTED_BLOCK_START]]';
const DECRYPTED_END = '[[SATCHEL_PGP_DECRYPTED_BLOCK_END]]';
const SMIME_DECRYPTED_START = '[[SATCHEL_SMIME_DECRYPTED_BLOCK_START]]';
const SMIME_DECRYPTED_END = '[[SATCHEL_SMIME_DECRYPTED_BLOCK_END]]';
const ENCRYPTED_BEGIN = '-----BEGIN PGP MESSAGE-----';
const ENCRYPTED_END = '-----END PGP MESSAGE-----';

export interface RenderSegment {
  kind: 'text' | 'pgp_decrypted' | 'pgp_encrypted' | 'smime_decrypted';
  content: string;
}

/**
 * Generic sentinel-fenced splitter.
 *
 * An unterminated opening marker is treated as text rather than as an open
 * region running to the end of the message: a truncated marker must not let
 * unverified content inherit the decrypted styling.
 */
function splitFencedSegments(
  input: string,
  startMarker: string,
  endMarker: string,
  kind: RenderSegment['kind'],
): RenderSegment[] {
  if (!input.includes(startMarker)) {
    return [{ kind: 'text', content: input }];
  }

  const output: RenderSegment[] = [];
  let cursor = 0;
  while (cursor < input.length) {
    const start = input.indexOf(startMarker, cursor);
    if (start < 0) {
      output.push({ kind: 'text', content: input.slice(cursor) });
      break;
    }

    if (start > cursor) {
      output.push({ kind: 'text', content: input.slice(cursor, start) });
    }

    const payloadStart = start + startMarker.length;
    const end = input.indexOf(endMarker, payloadStart);
    if (end < 0) {
      output.push({ kind: 'text', content: input.slice(start) });
      break;
    }

    output.push({ kind, content: input.slice(payloadStart, end).trim() });
    cursor = end + endMarker.length;
  }

  return output;
}

export function splitDecryptedSegments(input: string): RenderSegment[] {
  // Both markers are required here: the original guard checked for the closing
  // marker up front, so a body carrying only an opening marker stays text.
  if (!input.includes(DECRYPTED_START) || !input.includes(DECRYPTED_END)) {
    return [{ kind: 'text', content: input }];
  }
  return splitFencedSegments(input, DECRYPTED_START, DECRYPTED_END, 'pgp_decrypted');
}

export function splitSmimeDecryptedSegments(input: string): RenderSegment[] {
  return splitFencedSegments(input, SMIME_DECRYPTED_START, SMIME_DECRYPTED_END, 'smime_decrypted');
}

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

/**
 * Apply all three splitters in order, each one only to what the previous left
 * as text. Empty segments are dropped so the reader gets no stray separators.
 */
export function splitRenderSegments(text: string): RenderSegment[] {
  const expanded: RenderSegment[] = [];
  for (const segment of splitDecryptedSegments(text)) {
    if (segment.kind !== 'text') {
      expanded.push(segment);
      continue;
    }
    for (const smimeSegment of splitSmimeDecryptedSegments(segment.content)) {
      if (smimeSegment.kind !== 'text') {
        expanded.push(smimeSegment);
      } else {
        expanded.push(...splitEncryptedSegments(smimeSegment.content));
      }
    }
  }
  return expanded.filter((segment) => segment.content.trim().length > 0);
}
