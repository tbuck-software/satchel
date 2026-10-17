/**
 * Splits backend search snippets into renderable segments.
 *
 * `search_message_hits` wraps matched terms in the control characters U+0001
 * and U+0002 (chosen because they cannot survive the HTML sanitizer, so they
 * can never occur in stored mail text). Transport stays plain text — the
 * frontend renders segments through interpolation, never `v-html`.
 */

export interface HighlightSegment {
  text: string;
  marked: boolean;
}

const MARK_START = '\u0001';
const MARK_END = '\u0002';

export function splitHighlight(raw: string): HighlightSegment[] {
  const segments: HighlightSegment[] = [];
  let marked = false;
  let buffer = '';

  const flush = () => {
    if (buffer) {
      segments.push({ text: buffer, marked });
      buffer = '';
    }
  };

  for (const char of raw) {
    if (char === MARK_START) {
      flush();
      marked = true;
    } else if (char === MARK_END) {
      flush();
      marked = false;
    } else {
      buffer += char;
    }
  }
  flush();

  return segments;
}

/** Snippet text with the markers removed, for places that need plain text. */
export function stripHighlight(raw: string): string {
  return splitHighlight(raw)
    .map((segment) => segment.text)
    .join('');
}
