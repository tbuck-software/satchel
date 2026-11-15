/**
 * Strip every network reference from sender CSS: `url()`, `@import`, and the
 * string forms of `image-set()`, `image()` and `src()`. An allowlist — only
 * references that resolve inside the message survive. Mirrors `sanitize_css`
 * in `crates/satchel-core/src/email/sanitizer.rs`.
 */

/** Names a CSS escape could spell to reach a fetch the literal passes would miss. */
const ESCAPED_NAMES = new Set(['url', 'src', 'image', 'image-set', '-webkit-image-set', 'import', '@import']);
const STRING_IMAGE_FUNCTIONS = new Set(['src', 'image', 'image-set', '-webkit-image-set']);
const LOCAL_PREFIXES = ['data:', 'cid:', 'blob:', 'asset:', 'http://asset.localhost/', 'https://asset.localhost/'];

export function isLocalResourceUrl(url: string): boolean {
  const lower = url.trim().toLowerCase();
  return LOCAL_PREFIXES.some((prefix) => lower.startsWith(prefix));
}

function isLocalCssReference(value: string): boolean {
  const trimmed = value.trim();
  return trimmed.startsWith('#') || isLocalResourceUrl(trimmed);
}

// Length-preserving, unlike `toLowerCase()`, so indexes stay valid across both.
function asciiLower(value: string): string {
  return value.replace(/[A-Z]+/g, (run) => run.toLowerCase());
}

function isIdentChar(ch: string): boolean {
  return /[A-Za-z0-9_-]/.test(ch) || ch.charCodeAt(0) >= 0x80;
}

/** Decodes the escape whose backslash sits at `index`: `[text, end]`, or null when it is no escape. */
function readEscape(css: string, index: number): [string, number] | null {
  const next = css[index + 1];
  if (next === undefined || next === '\n' || next === '\r' || next === '\f') return null;
  if (!/[0-9a-fA-F]/.test(next)) {
    const code = css.codePointAt(index + 1)!;
    return [String.fromCodePoint(code), index + 1 + (code > 0xffff ? 2 : 1)];
  }
  let end = index + 1;
  while (end < css.length && end - index <= 6 && /[0-9a-fA-F]/.test(css[end]!)) end += 1;
  const code = parseInt(css.slice(index + 1, end), 16);
  if (css.startsWith('\r\n', end)) end += 2;
  else if (/[ \t\n\r\f]/.test(css[end] ?? '')) end += 1;
  const valid = code !== 0 && code <= 0x10ffff && !(code >= 0xd800 && code <= 0xdfff);
  return [valid ? String.fromCodePoint(code) : '�', end];
}

/** Spell escaped dangerous names out (`\75 rl(` → `url(`) so the literal passes see them. */
function normalizeEscapedNames(css: string): string {
  if (!css.includes('\\')) return css;
  let out = '';
  let index = 0;
  while (index < css.length) {
    const start = index;
    let decoded = '';
    let escaped = false;
    while (index < css.length) {
      const ch = css[index]!;
      if (isIdentChar(ch)) {
        decoded += ch;
        index += 1;
        continue;
      }
      const escape = ch === '\\' ? readEscape(css, index) : null;
      if (!escape) break;
      decoded += escape[0];
      index = escape[1];
      escaped = true;
    }
    if (index === start) {
      out += css[index];
      index += 1;
      continue;
    }
    const name = asciiLower(decoded);
    out += escaped && ESCAPED_NAMES.has(name) ? name : css.slice(start, index);
  }
  return out;
}

/** The index of the `)` closing the arguments opened at `open`, or -1, plus the strings inside. */
function scanArguments(css: string, open: number): { end: number; strings: string[] } {
  const strings: string[] = [];
  let depth = 1;
  let index = open + 1;
  while (index < css.length) {
    const ch = css[index]!;
    if (ch === '"' || ch === "'") {
      let end = index + 1;
      while (end < css.length && css[end] !== ch && css[end] !== '\n') {
        end += css[end] === '\\' ? 2 : 1;
      }
      strings.push(css.slice(index + 1, end));
      index = end + 1;
      continue;
    }
    if (ch === '/' && css[index + 1] === '*') {
      const close = css.indexOf('*/', index + 2);
      index = close === -1 ? css.length : close + 2;
      continue;
    }
    if (ch === '\\') {
      index += 2;
      continue;
    }
    if (ch === '(') depth += 1;
    else if (ch === ')' && --depth === 0) return { end: index, strings };
    index += 1;
  }
  return { end: -1, strings };
}

function stripImports(css: string): { css: string; blocked: number } {
  const haystack = asciiLower(css);
  let out = '';
  let cursor = 0;
  let blocked = 0;
  for (let start = haystack.indexOf('@import', cursor); start !== -1; start = haystack.indexOf('@import', cursor)) {
    out += css.slice(cursor, start);
    blocked += 1;
    const semicolon = haystack.indexOf(';', start);
    const brace = haystack.indexOf('}', start);
    if (brace !== -1 && (semicolon === -1 || brace < semicolon)) cursor = brace;
    else cursor = semicolon === -1 ? css.length : semicolon + 1;
  }
  return { css: out + css.slice(cursor), blocked };
}

function neutralizeStringImageFunctions(css: string): { css: string; blocked: number } {
  const haystack = asciiLower(css);
  let out = '';
  let cursor = 0;
  let blocked = 0;
  for (let open = css.indexOf('(', cursor); open !== -1; open = css.indexOf('(', open + 1)) {
    let start = open;
    while (start > cursor && isIdentChar(css[start - 1]!)) start -= 1;
    if (!STRING_IMAGE_FUNCTIONS.has(haystack.slice(start, open))) continue;
    const { end, strings } = scanArguments(css, open);
    if (end !== -1 && strings.every(isLocalCssReference)) continue;
    out += `${css.slice(cursor, start)}none`;
    blocked += 1;
    if (end === -1) return { css: out, blocked };
    cursor = end + 1;
    open = end;
  }
  return { css: out + css.slice(cursor), blocked };
}

function unquote(value: string): string {
  const trimmed = value.trim();
  const quote = trimmed[0];
  if ((quote === '"' || quote === "'") && trimmed.length > 1 && trimmed.endsWith(quote)) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

function neutralizeUrls(css: string): { css: string; blocked: number } {
  const haystack = asciiLower(css);
  let out = '';
  let cursor = 0;
  let blocked = 0;
  for (let start = haystack.indexOf('url(', cursor); start !== -1; start = haystack.indexOf('url(', cursor)) {
    const open = start + 'url('.length;
    const close = css.indexOf(')', open);
    out += css.slice(cursor, start);
    if (close === -1) {
      // An unterminated url() swallows the rest of the stylesheet.
      return { css: `${out}none`, blocked: blocked + 1 };
    }
    if (isLocalCssReference(unquote(css.slice(open, close)))) {
      out += css.slice(start, close + 1);
    } else {
      out += 'none';
      blocked += 1;
    }
    cursor = close + 1;
  }
  return { css: out + css.slice(cursor), blocked };
}

export function neutralizeRemoteCss(css: string): { css: string; blocked: number } {
  const imports = stripImports(normalizeEscapedNames(css));
  const functions = neutralizeStringImageFunctions(imports.css);
  const urls = neutralizeUrls(functions.css);
  return { css: urls.css, blocked: imports.blocked + functions.blocked + urls.blocked };
}
