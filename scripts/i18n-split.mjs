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
 *   closure mentions, minus what is already loaded whenever that import can
 *   run: startup, plus the chunks of the boundaries every importer sits in.
 *
 * "Mentions" is deliberately broad (see `keyMentions`): a key kept in a table
 * or assembled from a literal head counts in the module that holds the
 * literal. Such a string cannot reach `t()` before that module has loaded,
 * and loading the module loads its chunk first (`withMessages` in
 * `src/i18n/runtime.ts`, wired in by `scripts/vite-i18n-chunks.mjs`).
 *
 * What the analysis cannot see is a key whose text appears nowhere, e.g.
 * `` t(`${prefix}.label`) ``. `unanchoredKeyCalls` rejects that shape;
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
 * @typedef {{ id: string, file: string, keys: string[], available: number }} Chunk
 * @typedef {{
 *   startupKeys: string[],
 *   startupModules: Set<string>,
 *   modules: Set<string>,
 *   chunks: Map<string, Chunk>,
 *   closures: Map<string, Set<string>>,
 *   availableAt: Map<string, Set<string>>,
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

  // Lazy keys per boundary closure, startup already subtracted.
  /** @type {Map<string, Set<string>>} */
  const closures = new Map();
  /** @type {Map<string, Set<string>>} */
  const closureKeys = new Map();
  /** @type {Map<string, string[]>} */
  const regions = new Map();
  for (const boundary of importers.keys()) {
    const closure = staticClosure([boundary]);
    closures.set(boundary, closure);
    closureKeys.set(boundary, new Set([...mentions(closure)].filter((key) => !startup.has(key))));
    for (const file of closure) {
      if (!regions.has(file)) regions.set(file, []);
      /** @type {string[]} */ (regions.get(file)).push(boundary);
    }
  }

  /*
   * Keys loaded whenever a boundary's import can run, as a greatest fixpoint
   * over "intersection of what every importer has loaded". `null` stands for
   * "not known yet" (everything); startup modules have nothing lazy loaded.
   */
  /** @type {Map<string, Set<string> | null>} */
  const available = new Map();
  for (const boundary of importers.keys()) available.set(boundary, null);
  /** @type {(a: Set<string> | null, b: Set<string>) => Set<string>} */
  const intersect = (a, b) => (a === null ? new Set(b) : new Set([...a].filter((key) => b.has(key))));
  /** @param {string} file @returns {Set<string> | null} */
  const loadedIn = (file) => {
    if (startupModules.has(file)) return new Set();
    /** @type {Set<string> | null} */
    let loaded = null;
    for (const boundary of regions.get(file) ?? []) {
      const before = available.get(boundary);
      if (before === null || before === undefined) continue;
      loaded = intersect(loaded, new Set([...before, ...(closureKeys.get(boundary) ?? [])]));
    }
    return loaded;
  };
  for (let changed = true; changed;) {
    changed = false;
    for (const [boundary, from] of importers) {
      /** @type {Set<string> | null} */
      let next = null;
      for (const importer of from) {
        const loaded = loadedIn(importer);
        if (loaded !== null) next = intersect(next, loaded);
      }
      if (next === null) continue;
      const before = available.get(boundary);
      if (before === null || before === undefined || before.size !== next.size) {
        available.set(boundary, next);
        changed = true;
      }
    }
  }

  /** @type {Map<string, Chunk>} */
  const chunks = new Map();
  /** @type {Map<string, Set<string>>} */
  const availableAt = new Map();
  for (const boundary of importers.keys()) {
    const loaded = available.get(boundary) ?? new Set();
    availableAt.set(boundary, loaded);
    const own = [...(closureKeys.get(boundary) ?? [])].filter((key) => !loaded.has(key)).sort();
    chunks.set(boundary, { id: chunkId(boundary), file: boundary, keys: own, available: loaded.size });
  }

  return {
    startupKeys: [...startup].filter((key) => index.has(key)).sort(),
    startupModules,
    modules,
    chunks,
    closures,
    availableAt,
    unmentioned: unmentioned.sort(),
  };
}

const UNANCHORED_RE = /(?:\$t|\bt|\btc|\bte)\(\s*`\$\{/g;

/**
 * `t()` calls whose key starts with an interpolation: the analysis cannot
 * tell which keys they produce. Files listed in `UNANCHORED_KEYS` are
 * accepted, since their prefixes ship at startup.
 *
 * @param {Iterable<string>} files absolute paths
 * @returns {string[]} `path:line` of each offending call
 */
export function unanchoredKeyCalls(files) {
  /** @type {string[]} */
  const findings = [];
  for (const file of files) {
    const rel = path.relative(repoRoot, file);
    if (rel in UNANCHORED_KEYS) continue;
    const source = readFileSync(file, 'utf8');
    for (const match of source.matchAll(UNANCHORED_RE)) {
      const line = source.slice(0, match.index).split('\n').length;
      findings.push(`${rel}:${line}`);
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
