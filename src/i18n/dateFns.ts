import { de, enUS, type Locale } from 'date-fns/locale';
import type { AppLocale } from './locale';

const LOCALES: Record<AppLocale, Locale> = { en: enUS, de };

export function dateFnsLocale(locale: AppLocale): Locale {
  return LOCALES[locale] ?? enUS;
}
