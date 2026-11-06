/**
 * Splits the main window's catalogue into what startup needs and what can
 * load with the lazy code that uses it.
 *
 * The rule is derived from the module graph, not from a list of part files:
 *
 * - Startup carries every key a module in the static import graph of
 *   `src/main.ts` mentions, the keys Rust sends (`backend.*`, any key literal
 *   in the Rust sources), and keys no module of the window mentions at all —
 *   a key nobody writes down may still be built at runtime, so it stays.
 * - Every dynamic import target (`defineAsyncComponent`, router routes,
 *   `await import()`) is a boundary. Its chunk holds the keys its static
 *   closure mentions that are not in startup. Chunks overlap where views share
 *   components; subtracting what a parent chunk already loaded saved 2.8 KiB
 *   over all chunks and was the hardest part to get right, so it is gone.
 * - Boundaries in `FIRST_PAINT` mount on every start; their keys ship in
 *   startup instead of costing a chunk round trip before the first frame.
 *
 * "Mentions" is deliberately broad (see `keyMentions`): a key kept in a table
 * or assembled from a literal head counts in the module that holds the
 * literal. Such a string cannot reach `t()` before that module has loaded,
 * and loading the module loads its chunk first (`withMessages` in
 * `src/i18n/runtime.ts`, wired in by `scripts/vite-i18n-chunks.mjs`).
 *
 * What the analysis cannot see is a key whose text appears nowhere, e.g.
 * `` t(`${prefix}.label`) `` or `t(prefix + '.label')`. `unanchoredKeyCalls`
 * rejects both shapes;
 * `STARTUP.prefixes` is the explicit home for keys that must ship at startup
 * although no startup module names them.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import {
  keyIndex,
  keyMentions,
  messageKeys,
  moduleEdges,
  repoRoot,
  staticClosure,
} from './i18n-graph.mjs';

export const MAIN_ENTRY = 'src/main.ts';

/** The other windows' entries. Keys only they mention stay out of main. */
export const OTHER_ENTRIES = ['src/viewer/main.ts', 'src/tray/main.ts'];

export const STARTUP = {
  /*
   * Command errors arrive from Rust as `{ key, params }`, and any command,
   * including one a lazy view sends, reports through the startup toast stack.
   * Keeping the whole namespace is cheaper than proving which view can fail
   * with which key.
   */
  prefixes: ['backend'],
  /** Rust trees whose key literals (`"mail.foo"`, `"errors.x.{}"`) ship at startup. */
  rustSources: ['src-tauri/src', 'crates/satchel-core/src'],
};

/**
 * Call sites whose key has no literal head. Each needs an explicit home:
 * list the file with the key prefixes it can produce; those prefixes ship at
 * startup. Prefer rewriting the call so the key starts with a literal.
 *
 * @type {Record<string, string[]>}
 */
export const UNANCHORED_KEYS = {};

/**
 * Files whose key-shaped literals are not message keys handed to `t()`. Only
 * their `t('…')` calls count. Each entry says why.
 *
 * @type {Record<string, string>}
 */
export const NOT_KEYS = {
  // Section prefixes for the settings search, matched against the parts it
  // imports itself; listing them would put every settings string in the
  // sidebar's chunk.
  'src/components/settings/settingsSearch.ts': 'search index prefixes, read from the parts directly',
};

/**
 * Lazy components that mount on every start, before anyone clicks: the
 * sidebar's account rows and agenda, the connection banner, and the phone's
 * sync line and compose button. Their keys ship in startup. The raw-key crawl
 * (`tests/i18n-chunks.pw.ts`) fails when a cold start requests a chunk that is
 * not in `IDLE_PREFETCH` or explained there, which is how this list stays true.
 */
export const FIRST_PAINT = [
  'src/components/layout/ConnectionBanner.vue',
  'src/components/layout/SidebarAccounts.vue',
  'src/components/layout/SidebarAgenda.vue',
  'src/components/layout/SyncStatusLine.vue',
  'src/components/layout/ComposeFab.vue',
];

/**
 * Lazy code `bootstrap.ts` warms once the Inbox is usable (reader and command
 * palette, the two most common first actions). Their chunks load then, with
 * the code, after the first frame; `check-entry-bundles.mjs` reports them as
 * "after idle prefetch".
 */
export const IDLE_PREFETCH = [
  'src/components/mail/ThreadView.vue',
  'src/components/navigation/OmniSearch.vue',
];

const SRC = path.join(repoRoot, 'src');

/** @param {string} rel */
const abs = (rel) => path.join(repoRoot, rel);

/**
 * The keys a module can hand to `t()`: its mentions, or only its `t('…')`
 * calls for files in `NOT_KEYS`.
 *
 * @param {string} file absolute path
 * @param {ReturnType<typeof keyIndex>} index
 * @returns {Iterable<string>}
 */
export function moduleKeys(file, index) {
  if (!(path.relative(repoRoot, file) in NOT_KEYS)) return keyMentions(file, index);
  return [...messageKeys([file])].flatMap((key) => index.expand(key, false));
}

/** @param {string} file absolute path of a boundary module */
export function chunkId(file) {
  return path.relative(SRC, file).replace(/\.(vue|ts)$/, '').split(path.sep).join('.');
}

/** @param {string} dir @returns {string[]} */
function rustFiles(dir) {
  /** @type {string[]} */
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) out.push(...rustFiles(full));
    else if (name.endsWith('.rs')) out.push(full);
  }
  return out;
}

const RUST_KEY_RE = /"([a-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+\.?)(?:"|(\{))/g;

/** @param {ReturnType<typeof keyIndex>} index */
function rustKeys(index) {
  /** @type {Set<string>} */
  const keys = new Set();
  for (const dir of STARTUP.rustSources) {
    for (const file of rustFiles(abs(dir))) {
      for (const match of readFileSync(file, 'utf8').matchAll(RUST_KEY_RE)) {
        const literal = match[1];
        const open = match[2] !== undefined || literal.endsWith('.');
        for (const key of index.expand(literal, open)) keys.add(key);
      }
    }
  }
  return keys;
}

/**
 * @param {string[]} entries absolute paths
 * @returns {{ modules: Set<string>, importers: Map<string, Set<string>> }}
 */
function windowGraph(entries) {
  /** @type {Set<string>} */
  const modules = new Set();
  /** @type {Map<string, Set<string>>} */
  const importers = new Map();
  const queue = [...entries];
  while (queue.length) {
    const file = /** @type {string} */ (queue.pop());
    if (modules.has(file)) continue;
    modules.add(file);
    const edges = moduleEdges(file);
    for (const next of edges.static) queue.push(next);
    for (const next of edges.dynamic) {
      if (!importers.has(next)) importers.set(next, new Set());
      /** @type {Set<string>} */ (importers.get(next)).add(file);
      queue.push(next);
    }
  }
  return { modules, importers };
}

/**
 * @typedef {{ id: string, file: string, keys: string[] }} Chunk
 * @typedef {{
 *   startupKeys: string[],
 *   startupModules: Set<string>,
 *   modules: Set<string>,
 *   chunks: Map<string, Chunk>,
 *   closures: Map<string, Set<string>>,
 *   unmentioned: string[],
 * }} Split
 */

/**
 * @param {Iterable<string>} allKeys every key of the full catalogue
 * @param {{ startupRoots?: string[] }} [options] extra modules that load at
 *   startup, repo-relative; `dev:mock` passes the mock transport, which Vite
 *   swaps in for `services/backend/transport.ts`.
 * @returns {Split}
 */
export function computeSplit(allKeys, options = {}) {
  const keys = [...allKeys];
  const index = keyIndex(keys);
  /** @param {Iterable<string>} files */
  const mentions = (files) => {
    /** @type {Set<string>} */
    const out = new Set();
    for (const file of files) for (const key of moduleKeys(file, index)) out.add(key);
    return out;
  };

  const roots = [MAIN_ENTRY, ...(options.startupRoots ?? [])].map(abs);
  const { modules, importers } = windowGraph(roots);
  const startupModules = staticClosure(roots);
  const mainMentions = mentions(modules);
  const elsewhere = mentions([...windowGraph(OTHER_ENTRIES.map(abs)).modules]);

  const startup = mentions(startupModules);
  const rust = rustKeys(index);
  const unmentioned = [];
  for (const key of keys) {
    if (STARTUP.prefixes.some((prefix) => key === prefix || key.startsWith(`${prefix}.`))) startup.add(key);
    if (rust.has(key)) startup.add(key);
    if (!mainMentions.has(key) && !elsewhere.has(key)) {
      unmentioned.push(key);
      startup.add(key);
    }
  }
  for (const prefixes of Object.values(UNANCHORED_KEYS)) {
    for (const prefix of prefixes) for (const key of index.expand(prefix, true)) startup.add(key);
  }

  for (const file of FIRST_PAINT) {
    const boundary = abs(file);
    if (!importers.has(boundary)) throw new Error(`FIRST_PAINT names no dynamic import: ${file}`);
    for (const key of mentions(staticClosure([boundary]))) startup.add(key);
  }

  /** @type {Map<string, Set<string>>} */
  const closures = new Map();
  /** @type {Map<string, Chunk>} */
  const chunks = new Map();
  for (const boundary of importers.keys()) {
    const closure = staticClosure([boundary]);
    closures.set(boundary, closure);
    const own = [...mentions(closure)].filter((key) => !startup.has(key)).sort();
    chunks.set(boundary, { id: chunkId(boundary), file: boundary, keys: own });
  }

  return {
    startupKeys: [...startup].filter((key) => index.has(key)).sort(),
    startupModules,
    modules,
    chunks,
    closures,
    unmentioned: unmentioned.sort(),
  };
}

/** A call of `t`, `tc`, `te` or `$t`, bare or as a method (`i18n.global.t(`). */
const KEY_CALL_RE = /(?:\$t|(?<![\w$])(?:t|tc|te))\(/g;
/** A key head: at least one segment and a dot, e.g. `thread.state.`. */
const KEY_HEAD_RE = /^[a-z][A-Za-z0-9_]*\./;

/**
 * The first argument of a call whose `(` ends at `start`, as source text.
 *
 * @param {string} source
 * @param {number} start
 */
function firstArgument(source, start) {
  let depth = 0;
  let index = start;
  while (index < source.length) {
    const char = source[index];
    if (char === '"' || char === "'") {
      index += 1;
      while (index < source.length && source[index] !== char) index += source[index] === '\\' ? 2 : 1;
      index += 1;
      continue;
    }
    if (char === '`') {
      index = templateEnd(source, index) + 1;
      continue;
    }
    if ('([{'.includes(char)) depth += 1;
    else if (')]}'.includes(char)) {
      if (depth === 0) break;
      depth -= 1;
    } else if (char === ',' && depth === 0) break;
    index += 1;
  }
  return source.slice(start, index).trim();
}

/** @param {string} source @param {number} open index of the opening backtick */
function templateEnd(source, open) {
  let index = open + 1;
  let depth = 0;
  while (index < source.length) {
    const char = source[index];
    if (char === '\\') index += 2;
    else if (char === '$' && source[index + 1] === '{') { depth += 1; index += 2; }
    else if (char === '}' && depth > 0) { depth -= 1; index += 1; }
    else if (char === '`' && depth === 0) return index;
    else index += 1;
  }
  return index;
}

/**
 * Why a key argument hides its text from the analysis, or null when it does
 * not. A string literal or a template with a literal head names the key
 * itself. An identifier, member, table lookup, helper call or ternary carries
 * a literal written somewhere else, where the mention scan finds it. What is
 * left is a key assembled here from parts that are not a head: `p + '.title'`
 * or `` `${p}.title` ``.
 *
 * @param {string} argument
 * @returns {string | null}
 */
export function unanchoredReason(argument) {
  let index = 0;
  while (index < argument.length) {
    const char = argument[index];
    if (char === '"' || char === "'") {
      index += 1;
      while (index < argument.length && argument[index] !== char) index += argument[index] === '\\' ? 2 : 1;
      index += 1;
      continue;
    }
    if (char === '`') {
      const end = templateEnd(argument, index);
      const body = argument.slice(index + 1, end);
      if (body.includes('${') && !KEY_HEAD_RE.test(body)) return 'template without a literal key head';
      index = end + 1;
      continue;
    }
    if (char === '+' && argument[index + 1] !== '+' && argument[index - 1] !== '+') return 'key built with + (write a template with a literal head instead)';
    index += 1;
  }
  return null;
}

/**
 * `t()` calls whose key the analysis cannot place: see `unanchoredReason`.
 * Files listed in `UNANCHORED_KEYS` are accepted, since their prefixes ship
 * at startup.
 *
 * @param {Iterable<string>} files absolute paths
 * @returns {string[]} `path:line: reason` of each offending call
 */
export function unanchoredKeyCalls(files) {
  /** @type {string[]} */
  const findings = [];
  for (const file of files) {
    const rel = path.relative(repoRoot, file);
    if (rel in UNANCHORED_KEYS) continue;
    const source = readFileSync(file, 'utf8');
    for (const match of source.matchAll(KEY_CALL_RE)) {
      const start = /** @type {number} */ (match.index) + match[0].length;
      const reason = unanchoredReason(firstArgument(source, start));
      if (!reason) continue;
      const line = source.slice(0, match.index).split('\n').length;
      findings.push(`${rel}:${line}: ${reason}`);
    }
  }
  return findings;
}

if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) {
  // The generated full catalogue; importing i18n-merge here would be circular.
  const en = JSON.parse(readFileSync(abs('src/i18n/en.json'), 'utf8'));
  /** @param {unknown} value @param {string[]} trail @returns {string[]} */
  const leaves = (value, trail = []) => (typeof value === 'object' && value !== null
    ? Object.entries(value).flatMap(([key, child]) => leaves(child, [...trail, key]))
    : [trail.join('.')]);
  const split = computeSplit(leaves(en));
  process.stdout.write(`startup: ${split.startupKeys.length} keys (${split.startupModules.size} modules)\n`);
  const rows = [...split.chunks.values()].filter((chunk) => chunk.keys.length).sort((a, b) => b.keys.length - a.keys.length);
  for (const chunk of rows) process.stdout.write(`${chunk.keys.length}\t${chunk.id}\n`);
}
