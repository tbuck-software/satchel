import { createI18n, type LocaleMessageValue } from 'vue-i18n';
import { datetimeFormats, numberFormats } from './formats';
import {
  DEFAULT_LOCALE,
  readStoredLocalePreference,
  resolveLocale,
  type AppLocale,
} from './locale';
import { setActiveI18n } from './runtime';

export type LocaleMessages = Partial<Record<AppLocale, Record<string, LocaleMessageValue>>>;

/**
 * Composition mode only — `legacy: false`. `globalInjection` is on so templates
 * can call `$t()` without a `useI18n()` line in every component.
 */
export function createAppI18n(messages: LocaleMessages, locale?: AppLocale, loadLocale?: (locale: AppLocale) => Promise<void>) {
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

  setActiveI18n(instance.global as never, loadLocale);
  return instance;
}

/** Load the chosen catalogue before mounting; keep the other locale on demand. */
export function createLazyAppI18n(loaders: Record<AppLocale, () => Promise<{ default: Record<string, LocaleMessageValue> }>>) {
  const locale = resolveLocale(readStoredLocalePreference());
  const pending = new Map<AppLocale, Promise<void>>();
  function loadLocale(next: AppLocale): Promise<void> {
    const cached = pending.get(next);
    if (cached) return cached;
    const loading = loaders[next]().then(module => {
      instance.global.setLocaleMessage(next, module.default);
    }).catch(error => {
      pending.delete(next);
      throw error;
    });
    pending.set(next, loading);
    return loading;
  }
  const instance = createAppI18n({}, locale, loadLocale);
  return { instance, ready: loadLocale(locale) };
}
