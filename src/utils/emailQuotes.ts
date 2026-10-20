/**
 * Quote extraction for received mail.
 *
 * Splits a message body into "what this sender actually wrote" and the chain of
 * quoted replies below it, so the reader sees the new text first and the history
 * collapsed behind a disclosure.
 *
 * There is no standard for quoting. Every client marks history differently —
 * Gmail wraps it in `.gmail_quote`, Outlook in a `messageReplySection`, Apple
 * Mail in `blockquote[type="cite"]`, and plain-text mailers only prefix lines
 * with `>`. The selectors and patterns below are the accumulated shape of real
 * mail, not a specification, which is why this module changes often and why it
 * is separated from the component: it is the part worth testing.
 *
 * Two paths lead in:
 *   - `extractHtmlBody`  — DOM-based, for messages carrying text/html
 *   - `extractPlainTextBody` — line-based, for text/plain
 * Both end in the same normalisation (`mergeQuoteContinuations` +
 * `dedupeQuoteParts`), because the same reply chain often arrives twice, once
 * per MIME alternative.
 *
 * The German phrases below (`quoteIntroPattern`, `SIGN_OFF_PHRASES`,
 * `CONTACT_DETAIL_LINE`, …) parse *incoming* mail and stay untranslated — they
 * have to keep matching German messages whatever the UI language is. Only
 * `quoteLabel`'s fallback is text we generate for the reader.
 */
import { t } from '@/i18n/runtime';

export interface QuotePart {
  id: string;
  label: string;
  html?: string;
  text: string;
  normalizedHash: string;
  duplicateCount: number;
  hasIntro: boolean;
}

export interface BodyExtraction {
  currentHtml?: string;
  currentText: string;
  quotes: QuotePart[];
}

const quoteSelector = [
  'div[style*="border-left"]',
  '[name="messageReplySection"]',
  '[name="messageForwardSection"]',
  'blockquote',
  'blockquote[type="cite"]',
  '.gmail_quote',
  '.protonmail_quote',
  '[type="cite"]',
].join(',');

/**
 * The line that introduces a quote, in the spellings clients actually emit.
 *
 * `schrieb(?: .+)?:` covers both German word orders — "Am … schrieb:" and
 * "Am … schrieb Lena Becker:". The author-naming form is what Thunderbird and
 * Apple Mail send, and requiring the colon directly after "schrieb" used to
 * miss it, so German replies fell back to a numbered label and never
 * deduplicated by intro the way English ones did.
 */
const quoteIntroPattern =
  /^(on .+ wrote:|am .+ schrieb(?: .+)?:|.+ schrieb am .+:|nachricht von .+ am .+:?|[-_ ]*weitergeleitete nachricht[-_ ]*|von:|from:|gesendet:|sent:)/i;
const quoteDividerPattern =
  /^[-_\s]*(?:original message|urspruengliche nachricht|ursprüngliche nachricht)[-_\s]*$/i;
const quoteDividerInlinePattern =
  /[-_ \t]*(?:original message|urspruengliche nachricht|ursprüngliche nachricht)[-_ \t]*/gi;
const quoteFromHeaderPattern = /^(?:von|from):/i;
const quoteSentHeaderPattern = /^(?:gesendet|sent):/i;
const MAX_QUOTE_INTRO_LENGTH = 240;

/**
 * Flatten runaway indentation before display.
 *
 * Some mailers indent quoted history by a tab per level, which after a few
 * rounds pushes text off the right edge. Only kicks in past a threshold so
 * intentionally indented content (code, ASCII tables) survives untouched.
 */
export function normalizePlainTextIndent(input: string): string {
  const normalized = input.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
  const lines = normalized.split('\n');
  const excessiveIndentCount = lines.filter((line) => {
    const indent = line.match(/^[ \t]+/)?.[0] ?? '';
    return indent.replace(/\t/g, '    ').length >= 12 && line.trim().length > 0;
  }).length;

  if (excessiveIndentCount < 6) {
    return input;
  }

  return lines
    .map((line) => {
      const quoteMatch = line.match(/^((?:[ \t]*>[ \t]*)+)(.*)$/);
      if (quoteMatch) {
        // Both groups are part of the pattern above, so they always match.
        const quoteDepth = quoteMatch[1]!.split('>').length - 1;
        const cappedDepth = Math.min(quoteDepth, 3);
        return `${'> '.repeat(cappedDepth)}${quoteMatch[2]!.trimStart()}`;
      }

      const indent = line.match(/^[ \t]+/)?.[0] ?? '';
      if (indent.replace(/\t/g, '    ').length < 12 || !line.trim()) {
        return line;
      }
      return `    ${line.trimStart()}`;
    })
    .join('\n');
}

/**
 * HTML to text for quote comparison.
 *
 * Deliberately separate from the reader's plain-text view (`htmlToPlainText` in
 * `emailDocument.ts`): this one feeds hashing and deduplication, so it favours
 * stable output over readability and covers the block elements that carry quote
 * structure.
 */
export function textFromHtml(html: string): string {
  if (!html.trim()) {
    return '';
  }
  const parser = new DOMParser();
  const doc = parser.parseFromString(html, 'text/html');
  return doc.body ? normalizeQuoteText(quoteTextFromNode(doc.body)) : '';
}

const QUOTE_TEXT_BLOCKS = new Set([
  'address', 'article', 'aside', 'blockquote', 'div', 'footer', 'header', 'li', 'p', 'section', 'table', 'tr',
]);

/**
 * The raw text `textFromHtml` reads from a parsed tree: a line break per
 * `<br>` and after every block. It reads without touching the tree, so
 * `extractHtmlBody` can take the text from the document it parsed anyway and
 * go on to cut quotes out of it. Iterative, because mail nests deep enough to
 * exhaust the stack.
 */
function quoteTextFromNode(root: Node): string {
  const parts: string[] = [];
  const stack: Array<[node: Node, leaving: boolean]> = [[root, false]];
  while (stack.length > 0) {
    const [node, leaving] = stack.pop()!;
    if (leaving) {
      parts.push('\n');
      continue;
    }
    if (node.nodeType === 3 || node.nodeType === 4) {
      parts.push((node as CharacterData).data);
      continue;
    }
    if (node.nodeType !== 1) continue;
    const name = (node as Element).localName;
    if (name === 'br') {
      parts.push('\n');
      continue;
    }
    if (QUOTE_TEXT_BLOCKS.has(name)) stack.push([node, true]);
    for (let child = node.lastChild; child; child = child.previousSibling) {
      stack.push([child, false]);
    }
  }
  return parts.join('');
}

function normalizeQuoteText(text: string): string {
  return text
    .replace(/\u00a0/g, ' ')
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n[ \t]+/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

/**
 * Reduce a quote to a comparison key. Strips the parts that differ between two
 * copies of the same quote — marker depth, URLs rewritten by one client but not
 * the other, whitespace and case.
 */
export function normalizeForHash(input: string): string {
  return normalizePlainTextIndent(input)
    .replace(/^>+\s?/gm, '')
    .replace(/\bhttps?:\/\/\S+/gi, '')
    .replace(/\s+/g, ' ')
    .trim()
    .toLowerCase();
}

/** FNV-1a. Only needs to be stable and cheap — it identifies quotes, not secrets. */
export function stableHash(input: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, '0');
}

export function stripQuoteMarkers(input: string): string {
  return input
    .replace(/\r\n/g, '\n')
    .replace(/\r/g, '\n')
    .split('\n')
    .map((line) => line.replace(/^(?:[ \t]*>[ \t]*)+/, '').trimEnd())
    .map((line) => line.replace(quoteDividerInlinePattern, '\n'))
    .flatMap((line) => line.split('\n'))
    .filter((line) => !quoteDividerPattern.test(line.trim()))
    .join('\n')
    .trim();
}

/** The "On … wrote:" line that names who is being quoted, if the quote has one. */
export function quoteIntroFromText(text: string): string {
  const lines = stripQuoteMarkers(text)
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  return lines.find((line) => quoteIntroPattern.test(line)) ?? '';
}

function compactQuoteLabel(label: string): string {
  return label.length > 96 ? `${label.slice(0, 93)}...` : label;
}

export function quoteLabel(text: string, fallbackIndex: number, explicitIntro = ''): string {
  const intro = explicitIntro || quoteIntroFromText(text);
  if (intro) {
    return compactQuoteLabel(intro);
  }
  return t('mail.quotes.fallbackLabel', { index: fallbackIndex + 1 });
}

/**
 * Build a quote, or reject it. Fragments under 8 normalised characters are
 * quoting artefacts — a stray `>` or an empty blockquote — not history.
 */
export function buildQuotePart(
  input: { html?: string; text: string },
  fallbackIndex: number,
  explicitIntro = '',
): QuotePart | null {
  const text = stripQuoteMarkers(input.text || (input.html ? textFromHtml(input.html) : ''));
  const normalized = normalizeForHash(text);
  if (normalized.length < 8) {
    return null;
  }
  const normalizedHash = stableHash(normalized);
  const intro = explicitIntro || quoteIntroFromText(text);
  return {
    id: `quote-${normalizedHash}`,
    label: quoteLabel(text, fallbackIndex, intro),
    html: input.html,
    text,
    normalizedHash,
    duplicateCount: 1,
    hasIntro: Boolean(intro),
  };
}

function refreshQuotePart(part: QuotePart, fallbackIndex: number, explicitIntro = '') {
  const normalized = normalizeForHash(part.text);
  part.normalizedHash = stableHash(normalized);
  part.id = `quote-${part.normalizedHash}`;
  const intro = explicitIntro || quoteIntroFromText(part.text);
  part.label = quoteLabel(part.text, fallbackIndex, intro);
  part.hasIntro = Boolean(intro);
}

function appendTextToQuotePart(part: QuotePart, text: string) {
  const addition = stripQuoteMarkers(text);
  if (!addition) {
    return;
  }
  part.text = [part.text, addition].filter(Boolean).join('\n\n');
  part.html = undefined;
  refreshQuotePart(part, 0);
}

function prependTextToQuotePart(part: QuotePart, text: string, intro = '') {
  const prefix = stripQuoteMarkers(text);
  if (!prefix) {
    return;
  }
  part.text = [prefix, part.text].filter(Boolean).join('\n\n');
  part.html = undefined;
  refreshQuotePart(part, 0, intro);
}

/**
 * Sign-off phrases a signature can open with.
 *
 * The German forms are spelled out with and without umlauts because mail
 * arrives both ways, and the "Mit …" variants are listed separately: the match
 * is anchored at the start of the line, so "Mit freundlichen Grüßen" — the most
 * common German sign-off there is — does not match a pattern that begins at
 * "freundliche".
 */
const SIGN_OFF_PHRASES = [
  'mit freundlichen grüßen',
  'mit freundlichen gruessen',
  'mit besten grüßen',
  'mit besten gruessen',
  'mit herzlichen grüßen',
  'mit herzlichen gruessen',
  'freundliche grüße',
  'freundliche gruesse',
  'viele grüße',
  'viele gruesse',
  'beste grüße',
  'beste gruesse',
  'liebe grüße',
  'liebe gruesse',
  'herzliche grüße',
  'herzliche gruesse',
  'schöne grüße',
  'schoene gruesse',
  'grüße',
  'gruesse',
  'gruß',
  'gruss',
  'best regards',
  'kind regards',
  'warm regards',
  'best wishes',
  'kind wishes',
  'regards',
  'cheers',
  'sincerely',
  'yours sincerely',
  'yours faithfully',
  'yours truly',
  'many thanks',
  'thanks in advance',
  'thank you',
  'all the best',
  'take care',
  'looking forward',
  // The neighbours Satchel is most likely to correspond with. Each of these
  // is a fixed formula, not a phrase that starts ordinary prose.
  'met vriendelijke groet',
  'cordialement',
  'bien cordialement',
  'cordiali saluti',
  'distinti saluti',
  'saludos cordiales',
  'un saludo',
  'atenciosamente',
  'med vänliga hälsningar',
  'med venlig hilsen',
  'venlig hilsen',
  'ystävällisin terveisin',
  'z poważaniem',
  's pozdravem',
  'sent from',
  'gesendet von',
];

/**
 * Lines that read as contact details rather than prose. Generic on purpose —
 * a postal or legal-form keyword, an address, a URL, or a phone number.
 */
const CONTACT_DETAIL_LINE =
  /^(sent from|gesendet von|postfach|einzelunternehmen|geschäftsführer|geschaeftsfuehrer|ust-idnr|handelsregister|tel\.?[: ]|mobil|fax|mail@|e-?mail[: ]|https?:\/\/|[+\d][\d\s()/.-]{6,})/i;

/**
 * Short sign-offs, matched only as a line of their own.
 *
 * "Best", "Thanks" and the two-letter abbreviations are all common closings,
 * but as a prefix they would swallow ordinary sentences — "Best price so far",
 * "Thanks to Lena the build is green", "LG OLED bestellt". Requiring the whole
 * line keeps them usable.
 */
const SIGN_OFF_ABBREVIATION =
  /^(mfg|lg|vg|bg|vlg|hg|mvh|br|rgds|thx|best|thanks|danke|ciao)[\s,.!-]*$/i;

function escapeForPattern(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

const SIGN_OFF_START = new RegExp(
  `^(?:${SIGN_OFF_PHRASES.map(escapeForPattern).join('|')})`,
  'i',
);

/**
 * Does this fragment look like a signature rather than a quoted message?
 *
 * Signatures land between quotes and would otherwise be listed as messages of
 * their own. Recognised either by opening with a sign-off, or by being short
 * with a high density of contact-detail lines.
 *
 * Everything here is generic. An earlier version also matched a handful of
 * specific people and one company by name, which made detection work noticeably
 * better for one mailbox than for anybody else's — and silently so, since the
 * mailbox it was tuned for is the one it gets tested against.
 */
export function isSignatureLikeFragment(text: string): boolean {
  const lines = stripQuoteMarkers(text)
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length === 0 || quoteIntroFromText(text)) {
    return false;
  }

  const [firstLine = ''] = lines;
  if (SIGN_OFF_START.test(firstLine) || SIGN_OFF_ABBREVIATION.test(firstLine)) {
    return true;
  }

  const signatureHints = lines.filter((line) => CONTACT_DETAIL_LINE.test(line)).length;
  return lines.length <= 8 && signatureHints >= Math.max(2, Math.ceil(lines.length / 2));
}

/**
 * Detect a fragment that is really a signature followed by the next quote's
 * intro line, and report where to cut. Happens when a client's quote boundary
 * lands mid-fragment: the sign-off belongs to the message above, the intro to
 * the one below.
 */
export function splitSignatureIntroFragment(
  text: string,
): { before: string; after: string; intro: string } | null {
  const lines = stripQuoteMarkers(text).split('\n');
  const introIndex = lines.findIndex((line, index) => {
    if (index === 0) {
      return false;
    }
    return quoteIntroPattern.test(line.trim());
  });
  if (introIndex < 1) {
    return null;
  }

  const before = lines.slice(0, introIndex).join('\n').trim();
  const after = lines.slice(introIndex).join('\n').trim();
  const [introLine = ''] = lines.slice(introIndex, introIndex + 1);
  const intro = introLine.trim();
  if (!before || !after || !isSignatureLikeFragment(before)) {
    return null;
  }

  return { before, after, intro };
}

/** Reattach fragments that belong to the quote above them. */
export function mergeQuoteContinuations(parts: QuotePart[]): QuotePart[] {
  const output: QuotePart[] = [];
  for (let index = 0; index < parts.length; index += 1) {
    const previous = parts[index];
    if (!previous) continue;
    const part = { ...previous };
    const split = splitSignatureIntroFragment(part.text);
    const lastOutput = output.at(-1);
    if (split && lastOutput) {
      appendTextToQuotePart(lastOutput, split.before);
      const next = parts[index + 1];
      if (next) {
        prependTextToQuotePart(next, split.after, split.intro);
      } else {
        part.text = split.after;
        part.html = undefined;
        refreshQuotePart(part, index, split.intro);
        output.push(part);
      }
      continue;
    }

    const tail = output.at(-1);
    if (tail && !part.hasIntro && isSignatureLikeFragment(part.text)) {
      appendTextToQuotePart(tail, part.text);
      continue;
    }

    refreshQuotePart(part, output.length);
    output.push(part);
  }

  return output;
}

function normalizeQuoteLabelForDedupe(part: QuotePart): string {
  const label = part.label
    .toLowerCase()
    .replace(/<\s+/g, '<')
    .replace(/\s+>/g, '>')
    .replace(/\s+/g, ' ')
    .trim();

  if (quoteFromHeaderPattern.test(label)) {
    // "Von: Lena" alone is not an identity: the same sender can occur several
    // times in one thread. The adjacent Sent header distinguishes those mails
    // while still matching the HTML and text alternatives of the same mail.
    const sentHeader = part.text
      .split('\n')
      .map((line) => line.trim())
      .find((line) => quoteSentHeaderPattern.test(line));
    return sentHeader
      ? `${label}|${sentHeader.toLowerCase().replace(/\s+/g, ' ')}`
      : '';
  }

  if (/weitergeleitete nachricht|forwarded message/i.test(label)) {
    return '';
  }

  return label;
}

/**
 * Collapse repeats, counting them instead of listing them.
 *
 * Matched two ways: identical content (hash), or the same intro line (label).
 * The label pass catches the common case where the HTML and text alternatives
 * of one message differ in whitespace but quote the same reply.
 */
export function dedupeQuoteParts(parts: QuotePart[]): QuotePart[] {
  const byHash = new Map<string, QuotePart>();
  const byLabel = new Map<string, QuotePart>();
  for (const part of parts) {
    const existing = byHash.get(part.normalizedHash);
    if (existing) {
      existing.duplicateCount += 1;
      continue;
    }
    const labelKey = part.hasIntro ? normalizeQuoteLabelForDedupe(part) : '';
    const existingByLabel = labelKey ? byLabel.get(labelKey) : undefined;
    if (existingByLabel) {
      existingByLabel.duplicateCount += 1;
      continue;
    }
    const stored = { ...part };
    byHash.set(part.normalizedHash, stored);
    if (labelKey) {
      byLabel.set(labelKey, stored);
    }
  }
  return Array.from(byHash.values());
}

function isQuoteIntroNode(node: ChildNode | null): node is HTMLElement | Text {
  if (!node) {
    return false;
  }
  const text = node.textContent?.trim() || '';
  return text.length > 0 && text.length < MAX_QUOTE_INTRO_LENGTH && quoteIntroPattern.test(text);
}

function previousMeaningfulSibling(node: ChildNode): ChildNode | null {
  let sibling = node.previousSibling;
  while (sibling && !(sibling.textContent?.trim())) {
    sibling = sibling.previousSibling;
  }
  return sibling;
}

function isReplyWrapper(element: HTMLElement | null): element is HTMLElement {
  const name = element?.getAttribute('name')?.toLowerCase() ?? '';
  return name === 'messagereplysection' || name === 'messageforwardsection';
}

function collectIntroText(wrapper: HTMLElement, beforeNode: ChildNode): string {
  const parts: string[] = [];
  let totalLength = 0;
  for (const child of Array.from(wrapper.childNodes)) {
    if (child === beforeNode) {
      break;
    }
    const text = child.textContent?.trim();
    if (text) {
      parts.push(text);
      totalLength += text.length + (parts.length > 1 ? 1 : 0);
      if (totalLength >= MAX_QUOTE_INTRO_LENGTH) {
        return '';
      }
    }
  }
  return parts.join(' ').replace(/\s+/g, ' ').trim();
}

function cleanQuoteDividerNodes(root: HTMLElement) {
  const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes: Text[] = [];
  while (walker.nextNode()) {
    if (walker.currentNode instanceof Text) {
      nodes.push(walker.currentNode);
    }
  }

  for (const node of nodes) {
    const cleaned = node.nodeValue?.replace(quoteDividerInlinePattern, '') ?? '';
    if (cleaned.trim()) {
      node.nodeValue = cleaned;
    } else {
      node.remove();
    }
  }
}

const MAX_QUOTE_EXTRACTION_DEPTH = 64;

function extractQuotePartsFromContainer(
  container: ParentNode,
  parts: QuotePart[],
  depth = 0,
) {
  // A depth limit keeps malformed or adversarial markup from exhausting the
  // renderer. When reached, the still-nested subtree remains intact inside
  // its parent quote, so no mail content is discarded.
  if (depth >= MAX_QUOTE_EXTRACTION_DEPTH) {
    return;
  }

  let quoteNode = container.querySelector?.(quoteSelector) as HTMLElement | null;
  while (quoteNode) {
    let effectiveQuoteNode = quoteNode;
    let replyWrapper = isReplyWrapper(quoteNode.parentElement)
      ? quoteNode.parentElement
      : null;
    if (!replyWrapper && quoteNode.matches('div.protonmail_quote, div.gmail_quote')) {
      const nestedQuote = quoteNode.querySelector(
        ':scope > blockquote, :scope blockquote[type="cite"], :scope > [type="cite"]',
      );
      if (nestedQuote instanceof HTMLElement) {
        effectiveQuoteNode = nestedQuote;
        replyWrapper = quoteNode;
      }
    }

    const quoteClone = effectiveQuoteNode.cloneNode(true) as HTMLElement;
    const nestedParts: QuotePart[] = [];
    extractQuotePartsFromContainer(quoteClone, nestedParts, depth + 1);

    const wrapperIntro = replyWrapper ? collectIntroText(replyWrapper, effectiveQuoteNode) : '';
    const previousSibling = previousMeaningfulSibling(effectiveQuoteNode);
    const introNode = !wrapperIntro && isQuoteIntroNode(previousSibling)
      ? previousSibling
      : null;
    const rawIntroText = wrapperIntro || introNode?.textContent?.trim() || '';
    const introText = quoteIntroFromText(rawIntroText) || stripQuoteMarkers(rawIntroText);
    if (introText) {
      const intro = quoteClone.ownerDocument.createElement('div');
      intro.textContent = introText;
      quoteClone.prepend(intro);
    }
    if (introNode) {
      introNode.parentNode?.removeChild(introNode);
    }

    cleanQuoteDividerNodes(quoteClone);
    const html = quoteClone.outerHTML;
    const part = buildQuotePart({ html, text: textFromHtml(html) }, parts.length, introText);
    if (part) {
      parts.push(part);
    }
    parts.push(...nestedParts);

    // A cloned quote root is detached. Calling remove() on that root is a
    // no-op, which previously made this loop select the same descendant
    // forever for Outlook/Word reply markup. Always remove a descendant of
    // the current container so every iteration makes observable progress.
    const removalTarget = replyWrapper && replyWrapper !== container
      ? replyWrapper
      : effectiveQuoteNode;
    const parent = removalTarget.parentElement;
    removalTarget.remove();
    const nextQuoteNode = (parent ?? container).querySelector?.(quoteSelector) as HTMLElement | null;
    if (nextQuoteNode === quoteNode) {
      // Defensive fallback for unusual DOM implementations: retain the
      // extracted part but force removal of the exact node before continuing.
      quoteNode.remove();
      quoteNode = container.querySelector?.(quoteSelector) as HTMLElement | null;
    } else {
      quoteNode = nextQuoteNode;
    }
  }
}

/**
 * Split an HTML message. Quotes are lifted out of the DOM, and the plain-text
 * rendering is mined as well: clients that mark history only with `>` inside
 * HTML leave nothing for the selectors to find.
 */
export function extractHtmlBody(html: string): BodyExtraction {
  const parser = new DOMParser();
  const doc = parser.parseFromString(html, 'text/html');
  // Read the text before the quotes are cut out of the tree: it is the same
  // text `textFromHtml(html)` returns, without parsing the message again.
  const fullText = html.trim() ? normalizeQuoteText(quoteTextFromNode(doc.body)) : '';
  const parts: QuotePart[] = [];
  extractQuotePartsFromContainer(doc.body, parts);
  const textFallback = extractPlainTextBody(fullText).quotes;
  const htmlQuoteTexts = parts.map((part) => normalizeForHash(part.text));
  // A selector-backed HTML quote may contain older messages whose own wrapper
  // has no recognizable marker. The text fallback finds those headers too, but
  // rendering them separately would repeat content already present in the HTML.
  const uncoveredTextFallback = textFallback.filter((part) => {
    const text = normalizeForHash(part.text);
    return !htmlQuoteTexts.some((htmlText) => htmlText.includes(text));
  });
  const preservedStyles = Array.from(doc.head.querySelectorAll('style'))
    .map((style) => style.outerHTML)
    .join('');
  const currentHtml = `${preservedStyles}${doc.body.innerHTML}`.trim();
  return {
    currentHtml,
    currentText: '',
    quotes: dedupeQuoteParts(mergeQuoteContinuations([...parts, ...uncoveredTextFallback])),
  };
}

function isPlainQuoteBoundary(line: string): boolean {
  const trimmed = line.trim();
  const unquoted = trimmed.replace(/^(?:>\s*)+/, '').trim();
  return trimmed.startsWith('>') || quoteIntroPattern.test(unquoted);
}

function isPlainQuoteIntro(line: string): boolean {
  const unquoted = line.trim().replace(/^(?:>\s*)+/, '').trim();
  return quoteIntroPattern.test(unquoted);
}

function plainQuoteLineText(line: string): string {
  return line.trim().replace(/^(?:>\s*)+/, '').trim();
}

function sentHeaderContinuesFromHeader(line: string, active: string[]): boolean {
  if (!quoteSentHeaderPattern.test(plainQuoteLineText(line))) {
    return false;
  }
  const firstActiveLine = active
    .map(plainQuoteLineText)
    .find(Boolean);
  // Outlook's four-line header is one boundary. Treating "Gesendet:" as the
  // next boundary produced a header-only quote followed by the same mail again.
  return Boolean(firstActiveLine && quoteFromHeaderPattern.test(firstActiveLine));
}

/**
 * Split a text/plain message at the first quote boundary, then cut the history
 * into one part per intro line.
 *
 * The boundary search skips line 0 on purpose: a reply that opens with "Am …
 * schrieb:" would otherwise have an empty body.
 */
export function extractPlainTextBody(text: string): BodyExtraction {
  const normalized = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
  const lines = normalized.split('\n');
  const firstQuoteIndex = lines.findIndex((line, index) => index > 0 && isPlainQuoteBoundary(line));
  if (firstQuoteIndex < 0) {
    return {
      currentText: normalizePlainTextIndent(text),
      quotes: [],
    };
  }

  const currentText = normalizePlainTextIndent(lines.slice(0, firstQuoteIndex).join('\n').trim());
  const quoteBuffers: string[][] = [];
  let active: string[] = [];

  for (const line of lines.slice(firstQuoteIndex)) {
    if (
      isPlainQuoteIntro(line)
      && !sentHeaderContinuesFromHeader(line, active)
      && active.some((entry) => entry.trim())
    ) {
      quoteBuffers.push(active);
      active = [];
    }
    active.push(line);
  }
  if (active.some((entry) => entry.trim())) {
    quoteBuffers.push(active);
  }

  const parts = quoteBuffers
    .map((buffer, index) => buildQuotePart({ text: normalizePlainTextIndent(buffer.join('\n')) }, index))
    .filter((part): part is QuotePart => Boolean(part));

  return {
    currentText,
    quotes: dedupeQuoteParts(mergeQuoteContinuations(parts)),
  };
}
