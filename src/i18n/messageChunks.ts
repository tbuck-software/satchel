/**
 * Lazy code and its strings arrive together (ADR 0017). The build wraps every
 * dynamic import whose module needs keys outside the main window's startup
 * catalogue in `withMessages`, which merges that locale chunk into the active
 * instance before the import starts. Chunks are remembered: a locale switch
 * loads every chunk loaded so far in the new locale before it switches.
 *
 * A chunk's loaders come from a table the build loads on first use
 * (`scripts/vite-i18n-chunks.mjs`), so the entry does not carry two hashed
 * file names per lazy import. Only the main window loads this module; the
 * viewer and the tray have complete subsets of their own.
 */
import type { Composer } from 'vue-i18n';
import type { AppLocale } from './locale';
import { activeI18n, addLocaleLoader, wantedLocales } from './runtime';

type Messages = Parameters<Composer['mergeLocaleMessage']>[1];

/** One locale chunk per supported locale. */
export type MessageChunkLoaders = Record<AppLocale, () => Promise<{ default: Messages }>>;

const chunkLoaders = new Map<string, () => Promise<MessageChunkLoaders>>();
/** Per instance, so a new instance (tests) merges every chunk again. */
const merged = new WeakMap<object, Map<string, Promise<void>>>();

function loadChunk(id: string, locale: AppLocale): Promise<void> {
  const composer = activeI18n();
  const loaders = chunkLoaders.get(id);
  if (!composer || !loaders) return Promise.resolve();
  let loads = merged.get(composer);
  if (!loads) merged.set(composer, loads = new Map());
  const cacheKey = `${locale}\u0000${id}`;
  const cached = loads.get(cacheKey);
  if (cached) return cached;
  const cache = loads;
  const loading = loaders().then((table) => table[locale]()).then((module) => {
    composer.mergeLocaleMessage(locale, module.default);
  }).catch((error) => {
    if (cache.get(cacheKey) === loading) cache.delete(cacheKey);
    throw error;
  });
  cache.set(cacheKey, loading);
  return loading;
}

addLocaleLoader((locale) => Promise.all([...chunkLoaders.keys()].map((id) => loadChunk(id, locale))));

/**
 * Runs `load` once the chunk `id` is merged in every locale in use, so the
 * module it imports never meets a key it cannot resolve.
 */
export function withMessages<T>(id: string, loaders: () => Promise<MessageChunkLoaders>, load: () => Promise<T>): Promise<T> {
  chunkLoaders.set(id, loaders);
  return Promise.all(wantedLocales().map((locale) => loadChunk(id, locale))).then(load);
}
