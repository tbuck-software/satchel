/**
 * Whether an HTML body says nothing its text does not: paragraphs, line breaks
 * and plain wrappers, without styles, classes, links, images or tables. That
 * is what Satchel's own composer sends for a reply without formatting.
 *
 * Such a body read in the mail frame took the frame's system font, its padding
 * and the Original / Auto-dark / Text switch, so one's own one-line answer
 * looked as if it came from another program. As text it reads like the plain
 * messages around it.
 */
const PLAIN_TAG = /<\/?(?:p|br|div|span)\s*\/?>/gi;
const ANY_TAG = /<[a-z!/?]/i;
const MAX_LENGTH = 20_000;

/** A string test, not a parse: the renderer already parses each body twice. */
export function isFormattingFreeHtml(html: string): boolean {
  if (!html.trim() || html.length > MAX_LENGTH) return false;
  const rest = html.replace(PLAIN_TAG, '');
  if (ANY_TAG.test(rest)) return false;
  return rest.replace(/&nbsp;|&#160;/gi, ' ').trim().length > 0;
}

const ENTITIES: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ' };

/**
 * The text of a formatting-free body with its paragraphs kept apart. The
 * general HTML-to-text pass ends a paragraph with one line break, which ran
 * "Hi Tom," and the letter after it together.
 */
export function formattingFreeText(html: string): string {
  return html
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/p\s*>/gi, '\n\n')
    .replace(/<\/div\s*>/gi, '\n')
    .replace(PLAIN_TAG, '')
    .replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (entity, name: string) => {
      if (name[0] === '#') {
        const code = name[1]?.toLowerCase() === 'x' ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10);
        return Number.isFinite(code) ? String.fromCodePoint(code) : entity;
      }
      return ENTITIES[name.toLowerCase()] ?? entity;
    })
    .replace(/ /g, ' ')
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}
