import type { Attachment } from '@/types/mail';
import { blockRemoteImages } from '@/composables/useImagePolicy';
import {
  extractHtmlBody,
  extractPlainTextBody,
  normalizePlainTextIndent,
  type BodyExtraction,
  type QuotePart,
} from '@/utils/emailQuotes';
import { applyInlineAttachmentSources } from '@/utils/emailInlineImages';
import { buildIsolatedDocument, htmlToPlainText } from '@/utils/emailDocument';
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
 * reopened) reuses them instead of parsing again.
 */
export interface PreparedEmailBody {
  readonly extracted: BodyExtraction;
  readonly plainText: string;
  readonly markdownAvailable: boolean;
  document(options: DocumentOptions): PreparedDocument;
  quoteDocument(part: QuotePart, options: DocumentOptions): string;
}

interface CacheEntry {
  signature: string;
  html: string | undefined;
  text: string | undefined;
  body: PreparedEmailBody;
  size: number;
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
  const resolvedHtml = input.html
    ? applyInlineAttachmentSources(input.html, [...input.attachments], input.missingImageSource)
    : undefined;
  let extracted: BodyExtraction;
  if (!input.normalizeQuotes) {
    extracted = {
      currentHtml: resolvedHtml,
      currentText: normalizePlainTextIndent(input.text || ''),
      quotes: [],
    };
  } else if (resolvedHtml?.trim()) {
    extracted = extractHtmlBody(resolvedHtml);
  } else {
    extracted = extractPlainTextBody(input.text || '');
  }
  onGrow(extracted.currentHtml?.length ?? 0);

  let plainText: string | undefined;
  let markdownAvailable: boolean | undefined;
  const documents = new Map<string, PreparedDocument>();
  const quoteDocuments = new Map<string, string>();
  const optionKey = (options: DocumentOptions) =>
    `${options.autoDark ? 'dark' : 'light'}:${options.allowRemoteImages ? 'remote' : 'blocked'}`;

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
      const key = optionKey(options);
      let prepared = documents.get(key);
      if (!prepared) {
        const source = extracted.currentHtml || '';
        const blocked = source && !options.allowRemoteImages ? blockRemoteImages(source) : null;
        const safe = blocked?.html ?? source;
        prepared = {
          html: safe ? buildIsolatedDocument(safe, options.autoDark) : '',
          blockedCount: blocked?.blockedCount ?? 0,
        };
        documents.set(key, prepared);
        onGrow(prepared.html.length);
      }
      return prepared;
    },
    quoteDocument(part, options) {
      const key = `${part.id}:${optionKey(options)}`;
      let prepared = quoteDocuments.get(key);
      if (prepared === undefined) {
        const source = part.html ?? '';
        prepared = source
          ? buildIsolatedDocument(
              options.allowRemoteImages ? source : blockRemoteImages(source).html,
              options.autoDark,
            )
          : '';
        quoteDocuments.set(key, prepared);
        onGrow(prepared.length);
      }
      return prepared;
    },
  };
}

function evict(key: string) {
  const entry = cache.get(key);
  if (!entry) return;
  cachedChars -= entry.size;
  cache.delete(key);
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
  };
  entry.body = createPreparedBody(input, (chars) => {
    entry.size += chars;
    if (cache.get(key) === entry) {
      cachedChars += chars;
      enforceBudget(key);
    }
  });
  cache.set(key, entry);
  cachedChars += entry.size;
  enforceBudget(key);
  return entry.body;
}

export function preparedEmailBodyStats(): { entries: number; chars: number } {
  return { entries: cache.size, chars: cachedChars };
}

export function clearPreparedEmailBodies(): void {
  cache.clear();
  cachedChars = 0;
}
