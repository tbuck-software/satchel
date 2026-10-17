/**
 * Locale identity and preference storage.
 *
 * Deliberately free of `vue-i18n` and of any message import: the viewer and the
 * tray load this without pulling the client's message catalogue into their
 * bundles.
 */

import { createPersistedSlot, persistence } from '@/persistence';

export const SUPPORTED_LOCALES = ['en', 'de'] as const;

export type AppLocale = (typeof SUPPORTED_LOCALES)[number];

/** What the user picked. `system` follows the OS on every start. */
export type LocalePreference = AppLocale | 'system';

export const DEFAULT_LOCALE: AppLocale = 'en';

export const LOCALE_STORAGE_KEY = 'satchel.locale';

const localePreferenceSlot = createPersistedSlot<LocalePreference>(LOCALE_STORAGE_KEY, {
  fallback: () => 'system',
  decode: (stored) => (isLocalePreference(stored) ? stored : undefined),
  encode: (preference) => preference,
});

export function isAppLocale(value: unknown): value is AppLocale {
  return typeof value === 'string' && (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

export function isLocalePreference(value: unknown): value is LocalePreference {
  return value === 'system' || isAppLocale(value);
}

/** Maps a BCP 47 tag to a locale we ship; unknown languages fall back to `en`. */
export function localeFromLanguageTag(tag: string | undefined | null): AppLocale | null {
  if (!tag) return null;
  const primary = tag.toLowerCase().split('-')[0];
  return isAppLocale(primary) ? primary : null;
}

export function systemLocale(languages?: readonly string[]): AppLocale {
  const tags = languages
    ?? (typeof navigator === 'undefined'
      ? []
      : [...(navigator.languages ?? []), navigator.language].filter(Boolean) as string[]);
  for (const tag of tags) {
    const locale = localeFromLanguageTag(tag);
    if (locale) return locale;
  }
  return DEFAULT_LOCALE;
}

export function resolveLocale(
  preference: LocalePreference,
  languages?: readonly string[],
): AppLocale {
  return preference === 'system' ? systemLocale(languages) : preference;
}

export function readStoredLocalePreference(): LocalePreference {
  return persistence.read(localePreferenceSlot);
}

export function writeStoredLocalePreference(preference: LocalePreference) {
  persistence.write(localePreferenceSlot, preference);
}
