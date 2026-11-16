import { createI18n, type LocaleMessageValue } from 'vue-i18n';
import { datetimeFormats, numberFormats } from './formats';
import {
  DEFAULT_LOCALE,
  readStoredLocalePreference,
  resolveLocale,
  type AppLocale,
} from './locale';
import { loadLocaleMessages, setActiveI18n } from './runtime';

export type LocaleMessages = Partial<Record<AppLocale, Record<string, LocaleMessageValue>>>;

/**
 * Composition mode only — `legacy: false`. `globalInjection` is on so templates
 * can call `$t()` without a `useI18n()` line in every component.
 */
export function createAppI18n(messages: LocaleMessages, locale?: AppLocale, loadCatalogue?: (locale: AppLocale) => Promise<void>) {
  const instance = createI18n({
    legacy: false,
    globalInjection: true,
    locale: locale ?? resolveLocale(readStoredLocalePreference()),
    fallbackLocale: DEFAULT_LOCALE,
    messages,
    datetimeFormats,
    numberFormats,
    // Every key exists in `en`; a miss there is a bug, not a language gap.
    fallbackWarn: false,
  });

  setActiveI18n(instance.global as never, loadCatalogue);
  return instance;
}

/**
 * Load the chosen catalogue before mounting; keep the other locale on demand.
 * Merged rather than set: chunks of lazy code may already sit in that locale
 * (`withMessages` in `./runtime`), and setting would drop them.
 */
export function createLazyAppI18n(loaders: Record<AppLocale, () => Promise<{ default: Record<string, LocaleMessageValue> }>>) {
  const locale = resolveLocale(readStoredLocalePreference());
  const instance = createAppI18n({}, locale, (next) => loaders[next]().then((module) => {
    instance.global.mergeLocaleMessage(next, module.default);
  }));
  /*
   * Every key exists in both locales, so a miss means a key reached the
   * screen before its catalogue or chunk did. Production vue-i18n stays silent
   * about that; say it, so the raw-key crawl over the built app can see it.
   */
  instance.global.setMissingHandler((missingLocale, key) => {
    console.warn(`[i18n] missing key ${key} (${missingLocale})`);
  });
  return { instance, ready: loadLocaleMessages(locale) };
}
