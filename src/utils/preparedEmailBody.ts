import type { Attachment } from '@/types/mail';
import { blockRemoteImages } from '@/composables/useImagePolicy';
import {
  extractHtmlBody,
  extractPlainTextBody,
  normalizePlainTextIndent,
  type BodyExtraction,
  type QuotePart,
} from '@/utils/emailQuotes';
import { createInlineImageUrls, resolveInlineImageReferences } from '@/utils/emailInlineImages';
import { buildIsolatedDocument, htmlToPlainText } from '@/utils/emailDocument';
import { provisionalLinkAnswers, publicSuffixTableLoaded } from '@/utils/externalLinks';
import { hasMeaningfulMarkdown } from '@/utils/safeMarkdown';

export interface EmailBodyInput {
  html?: string;
  text?: string;
  attachments: readonly Attachment[];
  normalizeQuotes: boolean;
  /** Placeholder for a `cid:` image whose part is gone. */
  missingImageSource: string;
}

export interface DocumentOptions {
  autoDark: boolean;
  allowRemoteImages: boolean;
}

export interface PreparedDocument {
  html: string;
  blockedCount: number;
}

/**
 * Everything the reader derives from one body, computed on first use.
 *
 * Quote extraction, the plain-text fallback and the isolated document each
 * parse the HTML; a renderer that remounts (scrolled back into view, thread
 * reopened) reuses them instead of parsing again. Opening an HTML message in
 * its HTML view parses it twice: once for the quotes, once for the document.
 */
export interface PreparedEmailBody {
  readonly extracted: BodyExtraction;
  readonly plainText: string;
  /** Lexes `plainText` as Markdown on first read; for an HTML body that costs a parse of its own. */
  readonly markdownAvailable: boolean;
  document(options: DocumentOptions): PreparedDocument;
  quoteDocument(part: QuotePart, options: DocumentOptions): string;
  /**
   * Keeps the object URLs of the body's inline images alive until the returned
   * release runs. The cache holds the body while it is cached, a renderer while
   * it shows it; the URLs are revoked once neither does.
   */
  retain(): () => void;
}

interface CacheEntry {
  signature: string;
  html: string | undefined;
  text: string | undefined;
  body: PreparedEmailBody;
  size: number;
  release: () => void;
}

/**
 * A built document, and whether a link in it was assessed before the
 * public-suffix table arrived. Such a document is built again once the table
 * is there, so its link warnings do not outlive the wait.
 */
interface BuiltDocument<T> {
  value: T;
  provisional: boolean;
}

const MAX_ENTRIES = 48;
const MAX_CHARS = 24_000_000;
const cache = new Map<string, CacheEntry>();
let cachedChars = 0;

function attachmentSignature(input: EmailBodyInput): string {
  // Attachments only matter where the body points at one.
  if (!input.html || !/cid:/i.test(input.html)) return '';
  return input.attachments
    .map((attachment) => `${attachment.id}:${attachment.contentId ?? ''}:${attachment.dataUrl?.length ?? 0}`)
    .join('|');
}

function createPreparedBody(input: EmailBodyInput, onGrow: (chars: number) => void): PreparedEmailBody {
  // `cid:` references stay as they are through quote extraction, the image
  // blocker and the presentation passes; `buildIsolatedDocument` resolves them
  // last, in the tree it serializes anyway.
  let extracted: BodyExtraction;
  if (!input.normalizeQuotes) {
    extracted = {
      currentHtml: input.html,
      currentText: normalizePlainTextIndent(input.text || ''),
      quotes: [],
    };
  } else if (input.html?.trim()) {
    extracted = extractHtmlBody(input.html);
  } else {
    extracted = extractPlainTextBody(input.text || '');
  }
  onGrow(extracted.currentHtml?.length ?? 0);

  const inlineImages = createInlineImageUrls();
  const attachments = [...input.attachments];
  const resolveInlineImages = input.html && /cid:/i.test(input.html)
    ? (roots: Element[]) => {
      resolveInlineImageReferences(roots, attachments, {
        attachmentsResolved: true,
        missingSource: input.missingImageSource,
        keepPendingAttachments: false,
        sourceFor: inlineImages.sourceFor,
      });
    }
    : undefined;
  let holders = 0;

  let plainText: string | undefined;
  let markdownAvailable: boolean | undefined;
  const documents = new Map<string, BuiltDocument<PreparedDocument>>();
  const quoteDocuments = new Map<string, BuiltDocument<string>>();
  const optionKey = (options: DocumentOptions) =>
    `${options.autoDark ? 'dark' : 'light'}:${options.allowRemoteImages ? 'remote' : 'blocked'}`;

  function built<T>(
    cache: Map<string, BuiltDocument<T>>,
    key: string,
    build: () => T,
    size: (value: T) => number,
  ): T {
    const existing = cache.get(key);
    // The table check is a reactive read, so a computed showing a provisional
    // document evaluates again when the table arrives.
    if (existing && !(existing.provisional && publicSuffixTableLoaded())) return existing.value;
    const answersBefore = provisionalLinkAnswers();
    const value = build();
    cache.set(key, { value, provisional: provisionalLinkAnswers() !== answersBefore });
    if (!existing) onGrow(size(value));
    return value;
  }

  return {
    extracted,
    get plainText() {
      if (plainText === undefined) {
        const text = extracted.currentText;
        plainText = text.trim() ? text : htmlToPlainText(extracted.currentHtml || '');
      }
      return plainText;
    },
    get markdownAvailable() {
      markdownAvailable ??= hasMeaningfulMarkdown(this.plainText);
      return markdownAvailable;
    },
    document(options) {
      return built(documents, optionKey(options), () => {
        const source = extracted.currentHtml || '';
        const blocked = source && !options.allowRemoteImages ? blockRemoteImages(source) : null;
        const safe = blocked?.html ?? source;
        return {
          html: safe ? buildIsolatedDocument(safe, options.autoDark, resolveInlineImages) : '',
          blockedCount: blocked?.blockedCount ?? 0,
        };
      }, (prepared) => prepared.html.length);
    },
    quoteDocument(part, options) {
      return built(quoteDocuments, `${part.id}:${optionKey(options)}`, () => {
        const source = part.html ?? '';
        return source
          ? buildIsolatedDocument(
              options.allowRemoteImages ? source : blockRemoteImages(source).html,
              options.autoDark,
              resolveInlineImages,
            )
          : '';
      }, (prepared) => prepared.length);
    },
    retain() {
      holders += 1;
      let held = true;
      return () => {
        if (!held) return;
        held = false;
        holders -= 1;
        if (holders > 0) return;
        inlineImages.revoke();
        // Built documents point at the revoked URLs. Should the body be asked
        // again, it builds afresh, with data URLs.
        documents.clear();
        quoteDocuments.clear();
      };
    },
  };
}

function evict(key: string) {
  const entry = cache.get(key);
  if (!entry) return;
  cachedChars -= entry.size;
  cache.delete(key);
  entry.release();
}

function enforceBudget(protectedKey: string) {
  for (const key of cache.keys()) {
    if (cache.size <= MAX_ENTRIES && cachedChars <= MAX_CHARS) return;
    if (key !== protectedKey) evict(key);
  }
}

/**
 * The prepared body for one message, shared by every renderer that shows it.
 * Without a `cacheKey` (a file viewer, a preview) nothing is kept.
 */
export function prepareEmailBody(cacheKey: string | undefined, input: EmailBodyInput): PreparedEmailBody {
  if (!cacheKey) return createPreparedBody(input, () => {});

  const key = `${cacheKey}\u0000${input.normalizeQuotes ? 'quotes' : 'flat'}`;
  const signature = `${attachmentSignature(input)}\u0000${input.missingImageSource.length}`;
  const existing = cache.get(key);
  if (
    existing
    && existing.html === input.html
    && existing.text === input.text
    && existing.signature === signature
  ) {
    // Re-insert to keep the map in least-recently-used order.
    cache.delete(key);
    cache.set(key, existing);
    return existing.body;
  }
  evict(key);

  const entry: CacheEntry = {
    signature,
    html: input.html,
    text: input.text,
    size: 0,
    body: undefined as unknown as PreparedEmailBody,
    release: () => {},
  };
  entry.body = createPreparedBody(input, (chars) => {
    entry.size += chars;
    if (cache.get(key) === entry) {
      cachedChars += chars;
      enforceBudget(key);
    }
  });
  entry.release = entry.body.retain();
  cache.set(key, entry);
  cachedChars += entry.size;
  enforceBudget(key);
  return entry.body;
}

export function preparedEmailBodyStats(): { entries: number; chars: number } {
  return { entries: cache.size, chars: cachedChars };
}

export function clearPreparedEmailBodies(): void {
  const entries = [...cache.values()];
  cache.clear();
  cachedChars = 0;
  for (const entry of entries) entry.release();
}
