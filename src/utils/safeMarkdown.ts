import { marked, type Token, type Tokens } from 'marked';
import { assessExternalLink, parseMailtoLink } from './externalLinks';

export type SafeMarkdownInline =
  | { kind: 'text'; text: string }
  | { kind: 'strong'; children: SafeMarkdownInline[] }
  | { kind: 'emphasis'; children: SafeMarkdownInline[] }
  | { kind: 'delete'; children: SafeMarkdownInline[] }
  | { kind: 'code'; text: string }
  | { kind: 'link'; href: string; label: string; children: SafeMarkdownInline[] }
  | { kind: 'break' }
  | { kind: 'blocked_image'; alt: string; src?: string; width?: number; height?: number };

export type SafeMarkdownBlock =
  | { kind: 'paragraph'; children: SafeMarkdownInline[] }
  | { kind: 'heading'; depth: number; children: SafeMarkdownInline[] }
  | { kind: 'code_block'; text: string; language?: string }
  | { kind: 'blockquote'; children: SafeMarkdownBlock[] }
  | { kind: 'list'; ordered: boolean; start?: number; items: SafeMarkdownBlock[][] }
  | { kind: 'rule' }
  | {
      kind: 'table';
      align: Array<'center' | 'left' | 'right' | null>;
      header: SafeMarkdownInline[][];
      rows: SafeMarkdownInline[][][];
    }
  | { kind: 'raw_text'; text: string };

export interface SafeMarkdownDocument {
  blocks: SafeMarkdownBlock[];
}

interface SafeMarkdownOptions {
  baseUrl?: string;
}

const MARKED_OPTIONS = {
  async: false as const,
  breaks: true,
  gfm: true,
};
const MAX_MARKDOWN_INPUT_LENGTH = 250_000;

function inlineLabel(nodes: SafeMarkdownInline[]): string {
  return nodes.map((node) => {
    if (node.kind === 'text' || node.kind === 'code') return node.text;
    if (node.kind === 'blocked_image') return node.alt;
    if ('children' in node) return inlineLabel(node.children);
    return '';
  }).join('');
}

const IMAGE_DIMENSIONS_PATTERN = /^\{\s*width=(\d+)\s+height=(\d+)\s*\}/;
const MAX_IMAGE_DIMENSION = 10_000;

function takeImageDimensions(token: Token | undefined): {
  width?: number;
  height?: number;
  remainder?: string;
} {
  if (!token || token.type !== 'text' || typeof token.text !== 'string') return {};
  const match = IMAGE_DIMENSIONS_PATTERN.exec(token.text);
  if (!match) return {};

  const width = Number(match[1]);
  const height = Number(match[2]);
  if (width < 1 || height < 1 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION) {
    return {};
  }

  return { width, height, remainder: token.text.slice(match[0].length) };
}

function safeMarkdownImageSource(href: string, baseUrl?: string): string | undefined {
  const direct = assessExternalLink(href);
  if (direct.allowed) return direct.href;

  // GitLab notification mail keeps issue images as root-relative `/uploads/`
  // paths. Resolve only GitLab's randomized upload shapes, not arbitrary
  // relative links. Protocol-relative URLs remain forbidden because they can
  // switch to an unrelated host while looking like a relative path.
  const gitLabUpload = /^\/(?:uploads\/[a-z0-9_-]{32}|-\/project\/\d+\/uploads\/[a-z0-9_-]{32})\//i;
  if (!baseUrl || !gitLabUpload.test(href)) return undefined;
  try {
    const base = new URL(baseUrl);
    const resolved = new URL(href, base);
    if (resolved.origin !== base.origin) return undefined;
    const assessment = assessExternalLink(resolved.href);
    return assessment.allowed ? assessment.href : undefined;
  } catch {
    return undefined;
  }
}

export function inferMarkdownImageBaseUrl(input: string): string | undefined {
  // Notification bodies normally include their canonical issue/MR link. Its
  // origin makes this work for arbitrary self-hosted instances without a
  // GitLab-domain allow-list in Satchel.
  const candidates = input.match(/https?:\/\/[^\s<>()]+/gi) ?? [];
  for (const candidate of candidates) {
    try {
      const url = new URL(candidate.replace(/[.,;:!?]+$/, ''));
      if (url.protocol === 'https:' || url.protocol === 'http:') return url.origin;
    } catch {
      // Continue to the next absolute link in malformed mail text.
    }
  }
  return undefined;
}

function parseInline(
  tokens: Token[] | undefined,
  options: SafeMarkdownOptions,
): SafeMarkdownInline[] {
  if (!tokens) return [];
  const output: SafeMarkdownInline[] = [];

  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (!token) continue;
    switch (token.type) {
      case 'text': {
        const value = token as Tokens.Text;
        if (value.tokens?.length) output.push(...parseInline(value.tokens, options));
        else output.push({ kind: 'text', text: value.text });
        break;
      }
      case 'escape':
        output.push({ kind: 'text', text: (token as Tokens.Escape).text });
        break;
      case 'html':
        output.push({ kind: 'text', text: (token as Tokens.HTML).raw });
        break;
      case 'strong':
        output.push({ kind: 'strong', children: parseInline((token as Tokens.Strong).tokens, options) });
        break;
      case 'em':
        output.push({ kind: 'emphasis', children: parseInline((token as Tokens.Em).tokens, options) });
        break;
      case 'del':
        output.push({ kind: 'delete', children: parseInline((token as Tokens.Del).tokens, options) });
        break;
      case 'codespan':
        output.push({ kind: 'code', text: (token as Tokens.Codespan).text });
        break;
      case 'br':
        output.push({ kind: 'break' });
        break;
      case 'link': {
        const value = token as Tokens.Link;
        const children = parseInline(value.tokens, options);
        const label = inlineLabel(children) || value.text;
        const mailto = parseMailtoLink(value.href);
        const assessment = assessExternalLink(value.href, label);
        if (mailto) {
          output.push({ kind: 'link', href: mailto.href, label, children });
        } else if (assessment.allowed) {
          output.push({ kind: 'link', href: assessment.href, label, children });
        } else {
          output.push(...children);
        }
        break;
      }
      case 'image': {
        const value = token as Tokens.Image;
        const dimensions = takeImageDimensions(tokens[index + 1]);
        const blockedImage: SafeMarkdownInline = {
          kind: 'blocked_image',
          alt: value.text || 'Bild',
        };
        const src = safeMarkdownImageSource(value.href, options.baseUrl);
        if (src) blockedImage.src = src;
        if (dimensions.width && dimensions.height) {
          blockedImage.width = dimensions.width;
          blockedImage.height = dimensions.height;
        }
        output.push(blockedImage);
        if (dimensions.width && dimensions.height) {
          index += 1;
          if (dimensions.remainder) output.push({ kind: 'text', text: dimensions.remainder });
        }
        break;
      }
      default:
        if ('tokens' in token && Array.isArray(token.tokens)) {
          output.push(...parseInline(token.tokens, options));
        }
        else if ('text' in token && typeof token.text === 'string') output.push({ kind: 'text', text: token.text });
    }
  }

  return output;
}

function paragraphFromToken(
  token: Tokens.Paragraph | Tokens.Text,
  options: SafeMarkdownOptions,
): SafeMarkdownBlock {
  const children = parseInline(token.tokens, options);
  return {
    kind: 'paragraph',
    children: children.length > 0 ? children : [{ kind: 'text', text: token.text }],
  };
}

function parseBlocks(tokens: Token[], options: SafeMarkdownOptions): SafeMarkdownBlock[] {
  const output: SafeMarkdownBlock[] = [];

  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (!token) continue;
    switch (token.type) {
      case 'space':
      case 'def':
        break;
      case 'paragraph':
        output.push(paragraphFromToken(token as Tokens.Paragraph, options));
        break;
      case 'text':
        output.push(paragraphFromToken(token as Tokens.Text, options));
        break;
      case 'heading': {
        const value = token as Tokens.Heading;
        output.push({ kind: 'heading', depth: Math.min(3, Math.max(1, value.depth)), children: parseInline(value.tokens, options) });
        break;
      }
      case 'code': {
        const value = token as Tokens.Code;
        output.push({ kind: 'code_block', text: value.text, language: value.lang?.trim() || undefined });
        break;
      }
      case 'blockquote':
        output.push({ kind: 'blockquote', children: parseBlocks((token as Tokens.Blockquote).tokens, options) });
        break;
      case 'list': {
        const value = token as Tokens.List;
        output.push({
          kind: 'list',
          ordered: value.ordered,
          start: typeof value.start === 'number' ? value.start : undefined,
          items: value.items.map((item) => parseBlocks(item.tokens, options)),
        });
        break;
      }
      case 'hr':
        output.push({ kind: 'rule' });
        break;
      case 'table': {
        const value = token as Tokens.Table;
        output.push({
          kind: 'table',
          align: value.align,
          header: value.header.map((cell) => parseInline(cell.tokens, options)),
          rows: value.rows.map((row) => row.map((cell) => parseInline(cell.tokens, options))),
        });
        break;
      }
      case 'html':
        output.push({ kind: 'raw_text', text: (token as Tokens.HTML).raw });
        break;
      default:
        if ('tokens' in token && Array.isArray(token.tokens)) {
          output.push(...parseBlocks(token.tokens, options));
        }
        else if ('raw' in token && token.raw) output.push({ kind: 'raw_text', text: token.raw });
    }
  }

  return output;
}

export function parseSafeMarkdown(
  input: string,
  options: SafeMarkdownOptions = {},
): SafeMarkdownDocument {
  if (input.length > MAX_MARKDOWN_INPUT_LENGTH) {
    return { blocks: [{ kind: 'raw_text', text: input }] };
  }
  const resolvedOptions = {
    ...options,
    baseUrl: options.baseUrl ?? inferMarkdownImageBaseUrl(input),
  };
  return { blocks: parseBlocks(marked.lexer(input, MARKED_OPTIONS), resolvedOptions) };
}

function countRemoteImagesInInline(nodes: SafeMarkdownInline[]): number {
  return nodes.reduce((count, node) => {
    if (node.kind === 'blocked_image') return count + (node.src ? 1 : 0);
    if ('children' in node) return count + countRemoteImagesInInline(node.children);
    return count;
  }, 0);
}

function countRemoteImagesInBlocks(blocks: SafeMarkdownBlock[]): number {
  return blocks.reduce((count, block) => {
    switch (block.kind) {
      case 'paragraph':
      case 'heading':
        return count + countRemoteImagesInInline(block.children);
      case 'blockquote':
        return count + countRemoteImagesInBlocks(block.children);
      case 'list':
        return count + block.items.reduce(
          (itemCount, item) => itemCount + countRemoteImagesInBlocks(item),
          0,
        );
      case 'table':
        return count
          + block.header.reduce((cellCount, cell) => cellCount + countRemoteImagesInInline(cell), 0)
          + block.rows.reduce(
            (rowCount, row) => rowCount + row.reduce(
              (cellCount, cell) => cellCount + countRemoteImagesInInline(cell),
              0,
            ),
            0,
          );
      default:
        return count;
    }
  }, 0);
}

export function countRemoteMarkdownImages(input: string): number {
  return countRemoteImagesInBlocks(parseSafeMarkdown(input).blocks);
}

function containsMarkdownToken(tokens: Token[]): boolean {
  for (const token of tokens) {
    if (['heading', 'code', 'blockquote', 'list', 'hr', 'table'].includes(token.type)) return true;
    if (['strong', 'em', 'del', 'codespan', 'image'].includes(token.type)) return true;
    if (token.type === 'link' && token.raw.trim().startsWith('[')) return true;
    if ('tokens' in token && Array.isArray(token.tokens) && containsMarkdownToken(token.tokens)) return true;
    if (token.type === 'list') {
      const list = token as Tokens.List;
      if (list.items.some((item) => containsMarkdownToken(item.tokens))) return true;
    }
  }
  return false;
}

export function hasMeaningfulMarkdown(input: string): boolean {
  if (input.length > MAX_MARKDOWN_INPUT_LENGTH) return false;
  return containsMarkdownToken(marked.lexer(input, MARKED_OPTIONS));
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function inlineToHtml(nodes: SafeMarkdownInline[]): string {
  return nodes.map((node) => {
    switch (node.kind) {
      case 'text': return escapeHtml(node.text);
      case 'strong': return `<strong>${inlineToHtml(node.children)}</strong>`;
      case 'emphasis': return `<em>${inlineToHtml(node.children)}</em>`;
      case 'delete': return `<s>${inlineToHtml(node.children)}</s>`;
      case 'code': return `<code>${escapeHtml(node.text)}</code>`;
      case 'break': return '<br>';
      case 'blocked_image': {
        const dimensions = node.width && node.height
          ? ` style="width:${node.width}px;max-width:100%;aspect-ratio:${node.width}/${node.height}"`
          : '';
        return `<span data-blocked-markdown-image="true"${dimensions}>${escapeHtml(node.alt)}</span>`;
      }
      case 'link': return `<a href="${escapeHtml(node.href)}" target="_blank" rel="noopener noreferrer">${inlineToHtml(node.children)}</a>`;
    }
  }).join('');
}

function blocksToHtml(blocks: SafeMarkdownBlock[]): string {
  return blocks.map((block) => {
    switch (block.kind) {
      case 'paragraph': return `<p>${inlineToHtml(block.children)}</p>`;
      case 'heading': return `<h${block.depth}>${inlineToHtml(block.children)}</h${block.depth}>`;
      case 'code_block': return `<pre><code>${escapeHtml(block.text)}</code></pre>`;
      case 'blockquote': return `<blockquote>${blocksToHtml(block.children)}</blockquote>`;
      case 'list': {
        const tag = block.ordered ? 'ol' : 'ul';
        const start = block.ordered && block.start && block.start !== 1 ? ` start="${block.start}"` : '';
        return `<${tag}${start}>${block.items.map((item) => `<li>${blocksToHtml(item)}</li>`).join('')}</${tag}>`;
      }
      case 'rule': return '<hr>';
      case 'table': {
        const header = block.header.map((cell) => `<th>${inlineToHtml(cell)}</th>`).join('');
        const rows = block.rows.map((row) => `<tr>${row.map((cell) => `<td>${inlineToHtml(cell)}</td>`).join('')}</tr>`).join('');
        return `<table><thead><tr>${header}</tr></thead><tbody>${rows}</tbody></table>`;
      }
      case 'raw_text': return `<p>${escapeHtml(block.text)}</p>`;
    }
  }).join('');
}

export function safeMarkdownToHtml(document: SafeMarkdownDocument): string {
  return blocksToHtml(document.blocks);
}
