/**
 * Loads the main window's message chunks together with the lazy code that
 * needs them (ADR 0017).
 *
 * - Every dynamic `import('…')` of an app module whose boundary has a chunk
 *   (`scripts/i18n-split.mjs`) becomes `withMessages(id, () => import('…'))`:
 *   the chunk for each locale in use is merged before the import starts, so
 *   neither the module's top level nor its first render can meet a key it
 *   cannot resolve.
 * - The chunks are not files. `src/i18n/chunks/<id>.<locale>.json` is
 *   answered from the full catalogue on request, so they cannot go stale; the
 *   build precompiles them like any catalogue (`@intlify/unplugin-vue-i18n`
 *   includes that path). `chunks/table.js` maps each id to its two loaders and
 *   loads on first use; `chunks/with-messages.js` binds `withMessages` from
 *   `src/i18n/messageChunks.ts` to it. A call site in the entry names only the id.
 * - The build fails when the committed startup catalogue is stale, and when a
 *   module Rollup put into the startup graph is one the split thought lazy.
 */
import path from 'node:path';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import {
  DYNAMIC_IMPORT_RE,
  clearGraphCache,
  fsPath,
  keyIndex,
  repoRelative,
  repoRoot,
  resolveImport,
  splitFingerprint,
} from './i18n-graph.mjs';
import { buildBundles, keyPaths, pick, sortDeep } from './i18n-catalogue.mjs';
import { computeSplit, moduleKeys } from './i18n-split.mjs';

const LOCALES = ['en', 'de'];
const SRC = path.join(repoRoot, 'src');
const CHUNK_DIR = path.join(SRC, 'i18n', 'chunks');
const CHUNK_URL = '/src/i18n/chunks/';
const CHUNK_RE = /^(.+)\.([a-z]{2})\.json$/;
const HELPER = '__satchelWithMessages';
const HELPER_FILE = 'with-messages.js';
const TABLE_FILE = 'table.js';
const HELPER_IMPORT = `import { withMessages as ${HELPER} } from ${JSON.stringify(`${CHUNK_URL}${HELPER_FILE}`)};`;
const HELPER_SOURCE = `import { withMessages as load } from '@/i18n/messageChunks';
let table;
const loaders = (id) => (table ??= import(${JSON.stringify(`${CHUNK_URL}${TABLE_FILE}`)}).catch((error) => {
  table = undefined;
  throw error;
})).then((module) => module.default[id]);
export const withMessages = (id, importer) => load(id, () => loaders(id), importer);
`;

/**
 * @typedef {import('./i18n-split.mjs').Split} Split
 * @typedef {import('./i18n-split.mjs').Chunk} Chunk
 * @typedef {{ bundles: Record<string, Record<string, unknown>>, split: Split, byId: Map<string, Chunk> }} State
 */

/**
 * @param {{ mock?: boolean }} [options] `mock`: the dev:mock transport swap is
 *   active, so the mock's own imports load at startup too.
 * @returns {import('vite').Plugin[]}
 */
export function i18nChunks(options = {}) {
  const startupRoots = options.mock ? ['src/services/backend/mock.ts'] : [];
  /** @type {State | null} */
  let state = null;
  /** @type {'build' | 'serve'} */
  let command = 'build';

  /** @returns {State} */
  const compute = () => {
    clearGraphCache();
    const { bundles } = buildBundles();
    const split = computeSplit(keyPaths(bundles.en), { startupRoots });
    const byId = new Map([...split.chunks.values()].map((chunk) => [chunk.id, chunk]));
    return { bundles, split, byId };
  };
  const current = () => (state ??= compute());
  /** Which keys go where; a change re-cuts which imports are wrapped. */
  const layoutOf = (/** @type {State} */ next) => JSON.stringify([
    next.split.startupKeys,
    [...next.byId.values()].map((chunk) => [chunk.id, chunk.keys]),
  ]);
  /** The text every served catalogue holds; a copy edit changes only this. */
  const contentOf = (/** @type {State} */ next) => {
    const hash = createHash('sha1');
    for (const locale of LOCALES) {
      hash.update(JSON.stringify(pick(next.bundles[locale], next.split.startupKeys)));
      for (const chunk of next.byId.values()) hash.update(JSON.stringify(pick(next.bundles[locale], chunk.keys)));
    }
    return hash.digest('hex');
  };
  let layout = '';
  let content = '';
  /** @type {Map<string, string>} */
  const fingerprints = new Map();
  const remember = (/** @type {State} */ next) => {
    layout = layoutOf(next);
    content = contentOf(next);
    fingerprints.clear();
    for (const file of next.split.modules) fingerprints.set(file, splitFingerprint(file));
  };

  /** @param {Split} split */
  const staleStartup = (split) => {
    for (const locale of LOCALES) {
      const file = path.join(SRC, 'i18n', `startup.${locale}.json`);
      const committed = keyPaths(JSON.parse(readFileSync(file, 'utf8'))).sort();
      if (committed.join('\n') !== split.startupKeys.join('\n')) return true;
    }
    return false;
  };

  /** @type {import('vite').Plugin} */
  const serve = {
    name: 'satchel-i18n-chunks:serve',
    enforce: 'pre',
    configResolved(config) {
      command = config.command;
    },
    buildStart() {
      state = compute();
      if (command === 'serve') remember(state);
      if (options.mock || !staleStartup(state.split)) return;
      const message = 'src/i18n/startup.*.json is stale for the current code; run `pnpm i18n:build`.';
      if (command === 'build') this.error(message);
      else this.warn(message);
    },
    resolveId(id) {
      const file = id.startsWith(CHUNK_URL) ? path.join(repoRoot, id) : fsPath(id);
      return file.startsWith(`${CHUNK_DIR}${path.sep}`) ? file : null;
    },
    load(rawId) {
      const id = fsPath(rawId);
      const { bundles, split, byId } = current();
      if (id === path.join(CHUNK_DIR, HELPER_FILE)) return HELPER_SOURCE;
      if (id === path.join(CHUNK_DIR, TABLE_FILE)) {
        const rows = [...byId.values()].filter((chunk) => chunk.keys.length).map((chunk) => {
          const loaders = LOCALES
            .map((locale) => `${locale}: () => import(${JSON.stringify(`${CHUNK_URL}${chunk.id}.${locale}.json`)})`)
            .join(', ');
          return `  ${JSON.stringify(chunk.id)}: { ${loaders} },`;
        });
        return `export default {\n${rows.join('\n')}\n};\n`;
      }
      if (id.startsWith(`${CHUNK_DIR}${path.sep}`)) {
        const match = CHUNK_RE.exec(path.basename(id));
        const chunk = match ? byId.get(match[1]) : undefined;
        const locale = match?.[2];
        if (!chunk || !locale || !bundles[locale]) this.error(`unknown message chunk: ${id}`);
        return JSON.stringify(sortDeep(pick(bundles[/** @type {string} */ (locale)], chunk?.keys ?? [])));
      }
      // dev:mock loads the startup keys of the mock's imports as well.
      if (options.mock && id.startsWith(path.join(SRC, 'i18n', 'startup.'))) {
        const locale = /startup\.([a-z]{2})\.json$/.exec(id)?.[1];
        if (locale && bundles[locale]) return JSON.stringify(sortDeep(pick(bundles[locale], split.startupKeys)));
      }
      return null;
    },
    configureServer(server) {
      /*
       * A dev edit can move keys between startup and a chunk (code) or change
       * what a chunk says (parts). Either way the open page holds catalogues
       * cut from the old state, so reload it. Saves come in bursts; wait for
       * a quiet moment, and skip code saves that change neither imports nor
       * key literals, since those cannot move a key.
       */
      const PARTS = path.join(SRC, 'i18n', 'parts') + path.sep;
      /** @type {ReturnType<typeof setTimeout> | undefined} */
      let timer;
      let dirty = false;
      const recompute = () => {
        timer = undefined;
        if (!dirty) return;
        dirty = false;
        const next = compute();
        const nextLayout = layoutOf(next);
        const nextContent = contentOf(next);
        state = next;
        const moved = nextLayout !== layout;
        const edited = nextContent !== content;
        remember(next);
        if (!moved && !edited) return;
        if (moved && !options.mock && staleStartup(next.split)) {
          server.config.logger.warn('src/i18n/startup.*.json is stale; run `pnpm i18n:build`.');
        }
        if (moved) {
          server.moduleGraph.invalidateAll();
        } else {
          for (const module of server.moduleGraph.idToModuleMap.values()) {
            const id = fsPath(module.id ?? '');
            if (id.startsWith(CHUNK_DIR) || (options.mock && id.startsWith(path.join(SRC, 'i18n', 'startup.')))) {
              server.moduleGraph.invalidateModule(module);
            }
          }
        }
        server.ws.send({ type: 'full-reload' });
      };
      const refresh = (/** @type {string} */ changed) => {
        const file = fsPath(changed);
        if (file.startsWith(PARTS) && file.endsWith('.json')) {
          dirty = true;
        } else if (/\.(ts|vue)$/.test(file) && file.startsWith(SRC)) {
          const before = fingerprints.get(file);
          // Not in the graph: only an importer's edit can bring it in.
          if (before === undefined || before === splitFingerprint(file)) return;
          dirty = true;
        } else {
          return;
        }
        clearTimeout(timer);
        timer = setTimeout(recompute, 150);
      };
      server.watcher.on('change', refresh);
      server.watcher.on('add', refresh);
    },
  };

  /** @type {import('vite').Plugin} */
  const wrap = {
    name: 'satchel-i18n-chunks:wrap',
    transform(code, id) {
      // A `.vue` file's script may arrive inline or as its own
      // `?vue&type=script` request; styles and templates have no imports.
      const [rawFile, query = ''] = id.split('?');
      const file = fsPath(rawFile);
      if (query && !/(^|&)type=script(&|$)/.test(query)) return null;
      if (!file.startsWith(`${SRC}${path.sep}`) || !/\.(ts|vue)$/.test(file)) return null;
      if (file.startsWith(path.join(SRC, 'i18n') + path.sep) || /\.(test|spec)\.ts$/.test(file)) return null;
      if (!code.includes('import(')) return null;
      const { split } = current();
      // The viewer and tray carry complete subsets; only main window code waits for chunks.
      if (!split.modules.has(file)) return null;
      let wrapped = false;
      const next = code.replace(DYNAMIC_IMPORT_RE, (expression, _quote, spec) => {
        const target = resolveImport(spec, file);
        const chunk = target ? split.chunks.get(target) : undefined;
        if (!chunk?.keys.length) return expression;
        wrapped = true;
        return `${HELPER}(${JSON.stringify(chunk.id)}, () => ${expression})`;
      });
      // Same first line, so line numbers in the module stay where they were.
      return wrapped ? { code: `${HELPER_IMPORT}${next}`, map: null } : null;
    },
    generateBundle(_options, bundle) {
      const { bundles, split } = current();
      const index = keyIndex(keyPaths(bundles.en));
      const startup = new Set(split.startupKeys);
      /** @type {Map<string, Extract<(typeof bundle)[string], { type: 'chunk' }>>} */
      const chunks = new Map();
      for (const output of Object.values(bundle)) if (output.type === 'chunk') chunks.set(output.fileName, output);
      const entry = [...chunks.values()].find((chunk) => chunk.isEntry && fsPath(chunk.facadeModuleId ?? '').endsWith(`${path.sep}index.html`));
      if (!entry) return;
      /** @type {Set<string>} */
      const seen = new Set();
      const queue = [entry.fileName];
      /** @type {string[]} */
      const unexpected = [];
      while (queue.length) {
        const name = /** @type {string} */ (queue.pop());
        if (seen.has(name)) continue;
        seen.add(name);
        const chunk = chunks.get(name);
        if (!chunk) continue;
        queue.push(...chunk.imports);
        for (const moduleId of chunk.moduleIds) {
          const file = fsPath(moduleId.split('?')[0]);
          if (!file.startsWith(`${SRC}${path.sep}`) || !/\.(ts|vue)$/.test(file)) continue;
          if (split.startupModules.has(file)) continue;
          // The injected chunk loader and other key-free modules are harmless.
          const missing = [...moduleKeys(file, index)].filter((key) => !startup.has(key));
          if (missing.length) unexpected.push(`${repoRelative(file)} (${missing.slice(0, 3).join(', ')})`);
        }
      }
      if (unexpected.length) {
        this.error(
          `These modules load at startup, but the i18n split treats them as lazy, so these keys are `
            + `missing from src/i18n/startup.*.json. scripts/i18n-graph.mjs does not see how they are `
            + `imported:\n${[...new Set(unexpected)].sort().join('\n')}`,
        );
      }
    },
  };

  return [serve, wrap];
}
