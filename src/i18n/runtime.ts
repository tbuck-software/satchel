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
let loadActiveLocale: ((locale: AppLocale) => Promise<void>) | undefined;
let preferenceGeneration = 0;

export function setActiveI18n(composer: AnyComposer, loadLocale?: (locale: AppLocale) => Promise<void>) {
  ++preferenceGeneration;
  loadActiveLocale = loadLocale;
  active = composer;
  applyDocumentLocale(composer.locale.value as AppLocale);
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
  if (composer && !composer.availableLocales.includes(locale) && loadActiveLocale) {
    return loadActiveLocale(locale).then(apply).catch(error => console.warn('Locale catalogue could not be loaded', error));
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
