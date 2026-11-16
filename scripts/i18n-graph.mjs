/**
 * Module graph of an entry, plus the message keys it references.
 *
 * Used by `src/i18n/catalogue.test.ts` to prove that the viewer and tray
 * catalogues really cover the components those windows mount — the whole point
 * of the separate entries is that they do not import the client's catalogue,
 * so a missing key would only show up as a raw key on screen.
 *
 * `moduleEdges`, `staticClosure` and `keyMentions` serve the main window's
 * split catalogue (`scripts/i18n-split.mjs`), which has to tell a static
 * import (loads with its importer) from a dynamic one (loads later).
 */
import { readFileSync, existsSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/**
 * A repo-relative path written with `/` on every platform: the form the
 * tables in `i18n-split.mjs` are keyed by. `path.relative` writes `\` on
 * Windows, where a lookup with it misses every entry.
 *
 * @param {string} file absolute path
 */
export const repoRelative = (file) => path.relative(repoRoot, file).split(path.sep).join('/');

/**
 * A Vite module id as a path of this platform. Vite writes ids with `/` even
 * on Windows, while the split computes its paths with `path.join`; without
 * this, no id compares equal there and the release build cuts no chunks.
 * Virtual ids (`\0…`) stay as they are.
 *
 * @param {string} id
 */
export const fsPath = (id) => (id.startsWith('\0') ? id : path.normalize(id));

const CANDIDATE_SUFFIXES = ['', '.ts', '.vue', '/index.ts'];

/**
 * @param {string} spec
 * @param {string} fromFile
 * @returns {string | null}
 */
export function resolveImport(spec, fromFile) {
  const base = spec.startsWith('@/')
    ? path.join(repoRoot, 'src', spec.slice(2))
    : spec.startsWith('.')
      ? path.resolve(path.dirname(fromFile), spec)
      : null;
  if (!base) return null;
  for (const suffix of CANDIDATE_SUFFIXES) {
    // `/index.ts` on a `\` path would give a second spelling of the same file
    // on Windows, which no set lookup matches.
    const candidate = path.normalize(base + suffix);
    if (!/\.(ts|vue)$/.test(candidate)) continue;
    if (existsSync(candidate) && statSync(candidate).isFile()) return candidate;
  }
  return null;
}

const IMPORT_RE = /(?:from\s+|import\s*\(\s*)['"]([^'"]+)['"]/g;

/** @param {string[]} entries */
export function moduleGraph(entries) {
  const seen = new Set();
  const queue = entries.map((entry) => path.join(repoRoot, entry));

  while (queue.length) {
    const file = queue.pop();
    if (seen.has(file) || !existsSync(file)) continue;
    seen.add(file);
    const source = readFileSync(file, 'utf8');
    for (const match of source.matchAll(IMPORT_RE)) {
      const resolved = resolveImport(match[1], file);
      if (resolved && !seen.has(resolved)) queue.push(resolved);
    }
  }
  return [...seen].sort();
}

// Segments may hold `-` (`settings.sections.provider-features.label`).
const KEY_RE = /(?:\$t|\btc?)\(\s*(['"])([a-zA-Z][\w.-]*)\1/g;

/** @param {string[]} files */
export function messageKeys(files) {
  const keys = new Set();
  for (const file of files) {
    const source = readFileSync(file, 'utf8');
    for (const match of source.matchAll(KEY_RE)) keys.add(match[2]);
  }
  return keys;
}

const STATIC_IMPORT_RE = /(?:\bfrom\s*|\bimport\s+)['"]([^'"]+)['"]/g;
export const DYNAMIC_IMPORT_RE = /\bimport\s*\(\s*(['"])([^'"]+)\1\s*\)/g;

/** @type {Map<string, { static: string[], dynamic: string[] }>} */
const edgeCache = new Map();

/**
 * The app modules a file imports, split by how they load. Type-only imports
 * count as static: the graph may claim more for startup, never less.
 *
 * @param {string} file absolute path
 * @returns {{ static: string[], dynamic: string[] }}
 */
export function moduleEdges(file) {
  const cached = edgeCache.get(file);
  if (cached) return cached;
  const source = existsSync(file) ? readFileSync(file, 'utf8') : '';
  const staticEdges = new Set();
  const dynamicEdges = new Set();
  for (const match of source.matchAll(STATIC_IMPORT_RE)) {
    const resolved = resolveImport(match[1], file);
    if (resolved) staticEdges.add(resolved);
  }
  for (const match of source.matchAll(DYNAMIC_IMPORT_RE)) {
    const resolved = resolveImport(match[2], file);
    if (resolved) dynamicEdges.add(resolved);
  }
  const edges = { static: [...staticEdges].sort(), dynamic: [...dynamicEdges].sort() };
  edgeCache.set(file, edges);
  return edges;
}

/** Forget cached edges, for a dev server that keeps running while files change. */
export function clearGraphCache() {
  edgeCache.clear();
  mentionCache.clear();
}

/**
 * Every module that loads together with the given ones: static imports only.
 *
 * @param {string[]} files absolute paths
 * @returns {Set<string>}
 */
export function staticClosure(files) {
  /** @type {Set<string>} */
  const seen = new Set();
  const queue = [...files];
  while (queue.length) {
    const file = /** @type {string} */ (queue.pop());
    if (seen.has(file) || !existsSync(file)) continue;
    seen.add(file);
    for (const next of moduleEdges(file).static) if (!seen.has(next)) queue.push(next);
  }
  return seen;
}

/**
 * A key-shaped literal: `'mail.reply'`, `'outbox.status.'` (concatenated),
 * or the static head of a template literal `` `settings.sections.${id}` ``.
 * Group 3 is set when the literal continues with an interpolation.
 */
const KEY_LITERAL_RE = /(['"`])([a-z][A-Za-z0-9_-]*(?:\.[A-Za-z0-9_-]+)*\.?)(?:\1|(\$\{))/g;

/** @type {Map<string, Set<string>>} */
const mentionCache = new Map();

/**
 * Sorted key list with a prefix lookup.
 *
 * @param {Iterable<string>} keys
 */
export function keyIndex(keys) {
  const sorted = [...keys].sort();
  /** @param {string} prefix */
  const startingWith = (prefix) => {
    let low = 0;
    let high = sorted.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (sorted[mid] < prefix) low = mid + 1;
      else high = mid;
    }
    const out = [];
    for (let i = low; i < sorted.length && sorted[i].startsWith(prefix); i++) out.push(sorted[i]);
    return out;
  };
  return {
    /** @param {string} key */
    has: (key) => startingWith(key)[0] === key,
    /**
     * Keys a literal can stand for: the key itself and the leaves below it
     * (`'errors.smtp'` names a subtree), or, for an open head like
     * `settings.notifications.scope${…}`, every key starting with it.
     *
     * @param {string} literal
     * @param {boolean} open
     */
    expand: (literal, open) => {
      if (open) return startingWith(literal);
      const base = literal.endsWith('.') ? literal.slice(0, -1) : literal;
      return startingWith(base).filter((key) => key === base || key.startsWith(`${base}.`));
    },
  };
}

/**
 * Catalogue keys a file can produce. Broader than `messageKeys`: a key kept in
 * a table (`labelKey: 'folders.inbox'`) or assembled from a literal head
 * (`` t(`thread.state.${state}`) ``) counts where it is written down, since the
 * string cannot reach `t()` before the module holding it has loaded.
 *
 * @param {string} file absolute path
 * @param {ReturnType<typeof keyIndex>} index
 * @returns {Set<string>}
 */
export function keyMentions(file, index) {
  const cached = mentionCache.get(file);
  if (cached) return cached;
  const source = existsSync(file) ? readFileSync(file, 'utf8') : '';
  const keys = mentionsInSource(source, index);
  mentionCache.set(file, keys);
  return keys;
}

/**
 * What the split reads from a file: its import specifiers and its key-shaped
 * literals. An edit that leaves this unchanged cannot move a key, so a dev
 * server can skip recomputing the split for it.
 *
 * @param {string} file absolute path
 * @returns {string}
 */
export function splitFingerprint(file) {
  const source = existsSync(file) ? readFileSync(file, 'utf8') : '';
  const parts = [];
  for (const match of source.matchAll(STATIC_IMPORT_RE)) parts.push(`s:${match[1]}`);
  for (const match of source.matchAll(DYNAMIC_IMPORT_RE)) parts.push(`d:${match[2]}`);
  for (const match of source.matchAll(KEY_LITERAL_RE)) parts.push(`k:${match[2]}${match[3] ?? ''}`);
  for (const match of source.matchAll(KEY_RE)) parts.push(`t:${match[2]}`);
  return parts.join('\n');
}

/**
 * @param {string} source
 * @param {ReturnType<typeof keyIndex>} index
 * @returns {Set<string>}
 */
export function mentionsInSource(source, index) {
  /** @type {Set<string>} */
  const keys = new Set();
  for (const match of source.matchAll(KEY_LITERAL_RE)) {
    const literal = match[2];
    if (!literal.includes('.')) {
      // A top-level leaf; a bare namespace (`'settings'`) is not a key.
      if (index.has(literal)) keys.add(literal);
      continue;
    }
    const open = match[3] !== undefined || literal.endsWith('.');
    for (const key of index.expand(literal, open)) keys.add(key);
  }
  return keys;
}

if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) {
  const files = moduleGraph(process.argv.slice(2));
  process.stdout.write(files.map((file) => path.relative(repoRoot, file)).join('\n'));
  process.stdout.write(`\nTOTAL ${files.length}\n`);
}
