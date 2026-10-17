/**
 * Boil an email body down to the sentences that carry information: drop the
 * greeting line, quoted history, reply headers, and everything from the
 * signature or closing phrase onward. Lossy on purpose — used for previews
 * (e.g. collapsed cards in the topic feed), never for the full message view.
 */

const GREETING_RE =
  /^(hallo|hi|hey|moin|servus|guten\s+(morgen|tag|abend)|liebe[rs]?\b|sehr\s+geehrte|werte[rs]?\b|dear\b|hello|good\s+(morning|afternoon|evening))/i;

const GREETING_LINE_RE =
  /^(?:(?:hallo|hi|hey|moin|servus|liebe[rs]?|werte[rs]?|dear|hello)(?:\s+[\p{L}.'’()-]+){0,5}|guten\s+(?:morgen|tag|abend)(?:\s+[\p{L}.'’()-]+){0,4}|sehr\s+geehrte(?:r|s)?(?:\s+[\p{L}.'’()-]+){1,5}|good\s+(?:morning|afternoon|evening)(?:\s+[\p{L}.'’()-]+){0,4})[,!]?$/iu;

const CLOSING_RE =
  /^(mit\s+(freundlichen|besten|herzlichen)\s+grüßen|freundliche\s+grüße|viele\s+grüße|beste\s+grüße|liebe\s+grüße|herzliche\s+grüße|schöne\s+grüße|sonnige\s+grüße|gruß|grüße|lg|vg|mfg|bg|best\s+regards|kind\s+regards|warm\s+regards|regards|best\b|cheers|thanks|many\s+thanks|danke\s+(dir|euch|ihnen)\b|danke\s+und\s+(gruß|grüße)|vielen\s+dank\s+und\b)[\s,!.]*$/i;

/** "Am 22.07.2026 um 14:18 schrieb Mara:" / "On Jul 22, 2026, ... wrote:" */
const REPLY_HEADER_RE = /^(am\s.+schrieb\s.+:|on\s.+wrote:)$/i;

const SIGNATURE_DELIMITER_RE = /^--\s*$/;

const SIGNATURE_SELECTOR = [
  '.gmail_signature',
  '.moz-signature',
  '[data-smartmail="gmail_signature"]',
  '[id="Signature"]',
  '[id="signature"]',
].join(',');

function normalizedLine(value: string): string {
  return value.replace(/\u00a0/g, ' ').replace(/\s+/g, ' ').trim();
}

/**
 * Remove mail chrome while retaining the author's line breaks.
 * Unlike `mailGist`, this is suitable for the formatted topic view.
 */
export function trimMailBodyText(text: string): string {
  const lines = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n');
  let start = 0;
  while (start < lines.length && !lines[start]?.trim()) start += 1;
  if (start < lines.length && GREETING_LINE_RE.test(lines[start]!.trim())) start += 1;
  while (start < lines.length && !lines[start]?.trim()) start += 1;

  let end = lines.length;
  for (let index = start; index < lines.length; index += 1) {
    const line = lines[index]!.trim();
    if (
      SIGNATURE_DELIMITER_RE.test(line) ||
      CLOSING_RE.test(line) ||
      REPLY_HEADER_RE.test(line) ||
      line.startsWith('>')
    ) {
      end = index;
      break;
    }
  }
  while (end > start && !lines[end - 1]?.trim()) end -= 1;

  const trimmed = lines.slice(start, end).join('\n').trim();
  return trimmed || text.trim();
}

function removeFromNodeToEnd(node: Element, root: HTMLElement) {
  let current: Element | null = node;
  while (current && current !== root) {
    let sibling = current.nextSibling;
    while (sibling) {
      const next = sibling.nextSibling;
      sibling.remove();
      sibling = next;
    }
    const parent: HTMLElement | null = current.parentElement;
    if (current === node) current.remove();
    current = parent;
  }
}

/**
 * Remove greeting, signature and reply history without flattening HTML into a
 * text preview. Formatting inside the actual message remains untouched.
 */
export function trimMailBodyHtml(html: string): string {
  if (!html.trim()) return '';

  const doc = new DOMParser().parseFromString(html, 'text/html');
  const preservedStyles = Array.from(doc.head.querySelectorAll('style'))
    .map((style) => style.outerHTML)
    .join('');

  for (const signature of Array.from(doc.body.querySelectorAll(SIGNATURE_SELECTOR))) {
    signature.remove();
  }

  const lineElements = Array.from(
    doc.body.querySelectorAll<HTMLElement>('address, div, footer, p'),
  );
  const greeting = lineElements.find((element) =>
    GREETING_LINE_RE.test(normalizedLine(element.textContent || '')),
  );
  greeting?.remove();

  const signatureStart = lineElements.find((element) => {
    if (!element.isConnected) return false;
    const line = normalizedLine(element.textContent || '');
    return SIGNATURE_DELIMITER_RE.test(line) || CLOSING_RE.test(line);
  });
  if (signatureStart) removeFromNodeToEnd(signatureStart, doc.body);

  return `${preservedStyles}${doc.body.innerHTML}`.trim() || html.trim();
}

export function mailGist(text: string): string {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const kept: string[] = [];
  let seenContent = false;

  for (const raw of lines) {
    const line = raw.trim();

    // Signature block or a closing phrase ends the informative part.
    if (SIGNATURE_DELIMITER_RE.test(line)) break;
    if (seenContent && CLOSING_RE.test(line)) break;

    // Quoted history and the header introducing it carry no new information.
    if (line.startsWith('>') || REPLY_HEADER_RE.test(line)) continue;

    // The greeting is only a greeting while nothing substantial came before it.
    if (!seenContent && (line === '' || GREETING_RE.test(line))) continue;

    if (line !== '') seenContent = true;
    kept.push(line);
  }

  const gist = kept.join(' ').replace(/\s+/g, ' ').trim();
  return gist || text.replace(/\s+/g, ' ').trim();
}
