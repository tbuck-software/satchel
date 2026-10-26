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

/** Presentation only: operators still go unchanged to the backend parser. */
export function searchHighlightTerms(query: string): string[] {
  return (query.match(/(?:[^\s"]*"[^"]*"|\S+)/g) ?? []).flatMap((token) => {
    if (token.startsWith('-')) return [];
    if (token.includes(':') && !token.startsWith('subject:')) return [];
    const text = token.replace(/^subject:/, '').replace(/"/g, '').replace(/\*$/, '');
    return text ? [text] : [];
  });
}

/** Literal ranges, rendered as text nodes so mail text can never become HTML. */
export function highlightMatches(text: string, terms: string[]): HighlightSegment[] {
  const escaped = [...new Set(terms.filter(Boolean))]
    .sort((left, right) => right.length - left.length)
    .map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  if (!escaped.length) return [{ text, marked: false }];
  const pattern = new RegExp(escaped.join('|'), 'giu');
  const segments: HighlightSegment[] = [];
  let cursor = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index > cursor) segments.push({ text: text.slice(cursor, match.index), marked: false });
    segments.push({ text: match[0], marked: true });
    cursor = match.index + match[0].length;
  }
  if (cursor < text.length) segments.push({ text: text.slice(cursor), marked: false });
  return segments;
}
