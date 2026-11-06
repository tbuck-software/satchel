/**
 * Translation entry point for code that runs outside a component `setup()`:
 * stores, composables, plain utilities.
 *
 * The active instance is registered by whichever entry booted — client, viewer
 * or tray. Importing this module therefore costs nothing but the helpers; the
 * message catalogue stays with the entry that owns it, which is what keeps the
 * viewer and tray bundles from pulling in the client's ~1000 strings.
 */
import type { Composer } from 'vue-i18n';
import {
  DEFAULT_LOCALE,
  resolveLocale,
  writeStoredLocalePreference,
  type AppLocale,
  type LocalePreference,
} from './locale';

type AnyComposer = Composer<Record<string, unknown>, Record<string, unknown>, Record<string, unknown>, string, string, string>;

let active: AnyComposer | null = null;
let loadActiveCatalogue: ((locale: AppLocale) => Promise<void>) | undefined;
let preferenceGeneration = 0;

/*
 * More than the base catalogue can belong to a locale: the main window adds
 * the chunks of lazy code loaded so far (`./messageChunks`). Those register
 * here, so a locale switch waits for them too.
 */
const extraLoaders = new Set<(locale: AppLocale) => Promise<unknown>>();
let localeLoads = new Map<AppLocale, Promise<void>>();
/** Locales whose catalogue was requested; later chunks load in each of them. */
let wanted = new Set<AppLocale>();
/** Locales whose catalogue and every registered extra are merged. */
let ready = new Set<AppLocale>();

/**
 * Makes `composer` the instance stores and utilities translate with.
 * `loadCatalogue` merges a locale's base catalogue into it; without one the
 * instance is expected to carry its messages already.
 */
export function setActiveI18n(composer: AnyComposer, loadCatalogue?: (locale: AppLocale) => Promise<void>) {
  ++preferenceGeneration;
  loadActiveCatalogue = loadCatalogue;
  active = composer;
  localeLoads = new Map();
  wanted = new Set();
  ready = new Set();
  applyDocumentLocale(composer.locale.value as AppLocale);
}

/** Registers more messages that every locale load has to include. */
export function addLocaleLoader(load: (locale: AppLocale) => Promise<unknown>) {
  extraLoaders.add(load);
}

/** The locales a newly registered extra has to load in. */
export function wantedLocales(): AppLocale[] {
  return wanted.size ? [...wanted] : [currentLocale()];
}

/**
 * Loads a locale's catalogue and every registered extra. Cached: a second
 * call for the same locale returns the first promise.
 */
export function loadLocaleMessages(locale: AppLocale): Promise<void> {
  const cached = localeLoads.get(locale);
  if (cached) return cached;
  const loads = localeLoads;
  const done = ready;
  const wantedNow = wanted;
  wantedNow.add(locale);
  const loading = Promise.all([
    loadActiveCatalogue?.(locale),
    ...[...extraLoaders].map((load) => load(locale)),
  ]).then(() => {
    done.add(locale);
  }).catch((error) => {
    if (loads.get(locale) === loading) {
      loads.delete(locale);
      // A locale that failed to load is not in use; later chunks skip it.
      wantedNow.delete(locale);
    }
    throw error;
  });
  loads.set(locale, loading);
  return loading;
}

export function activeI18n(): AnyComposer | null {
  return active;
}

function applyDocumentLocale(locale: AppLocale) {
  if (typeof document !== 'undefined') document.documentElement.lang = locale;
}

export function currentLocale(): AppLocale {
  return (active?.locale.value as AppLocale) ?? DEFAULT_LOCALE;
}

/**
 * Applies a preference. `system` re-reads the OS languages, so the caller does
 * not have to resolve it.
 */
export function applyLocalePreference(preference: LocalePreference, persist = true) {
  const locale = resolveLocale(preference);
  const composer = active;
  const generation = ++preferenceGeneration;
  const apply = () => {
    if (generation !== preferenceGeneration || composer !== active) return;
    if (composer) composer.locale.value = locale;
    applyDocumentLocale(locale);
  };
  if (persist) writeStoredLocalePreference(preference);
  if (composer && !ready.has(locale) && (loadActiveCatalogue || extraLoaders.size)) {
    return loadLocaleMessages(locale).then(apply).catch(error => console.warn('Locale catalogue could not be loaded', error));
  }
  apply();
  return Promise.resolve();
}

/** Translate. Falls back to the key itself before an instance exists. */
export function t(key: string, named?: Record<string, unknown>): string {
  if (!active) return key;
  return named === undefined ? active.t(key) : active.t(key, named);
}

/** Whether the active catalogue carries the key. False before an instance exists. */
export function te(key: string): boolean {
  return active?.te(key) ?? false;
}

/** Translate with a count, using vue-i18n's plural forms. */
export function tc(key: string, count: number, named?: Record<string, unknown>): string {
  if (!active) return key;
  return active.t(key, { count, n: count, ...named }, count);
}

export function d(value: Date | number, format?: string): string {
  if (!active) return new Date(value).toISOString();
  return format ? active.d(value, format) : active.d(value);
}

export function n(value: number, format?: string): string {
  if (!active) return String(value);
  return format ? active.n(value, format) : active.n(value);
}
