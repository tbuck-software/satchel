import { currentLocale, t } from '@/i18n/runtime';
import type { AppLocale } from '@/i18n/locale';

const DAY_MS = 86_400_000;

const TIME_OPTIONS: Intl.DateTimeFormatOptions = {
  hour: '2-digit',
  minute: '2-digit',
  hour12: false,
};
const WEEKDAY_OPTIONS: Intl.DateTimeFormatOptions = { weekday: 'short' };
const DATE_OPTIONS: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short' };
const DATE_WITH_YEAR_OPTIONS: Intl.DateTimeFormatOptions = {
  day: 'numeric',
  month: 'short',
  year: 'numeric',
};
const MESSAGE_TIMESTAMP_OPTIONS: Intl.DateTimeFormatOptions = {
  day: 'numeric',
  month: 'short',
  hour: '2-digit',
  minute: '2-digit',
  hour12: false,
};

/**
 * `Intl.DateTimeFormat` construction is the expensive part, and this runs once
 * per visible row of a virtualised list — so formatters are built once per
 * locale and options object, not per call.
 */
const formatters = new Map<string, Intl.DateTimeFormat>();

export function localeFormatter(
  key: string,
  options: Intl.DateTimeFormatOptions,
  locale: AppLocale = currentLocale(),
): Intl.DateTimeFormat {
  const cacheKey = `${locale}:${key}`;
  let formatter = formatters.get(cacheKey);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(locale, options);
    formatters.set(cacheKey, formatter);
  }
  return formatter;
}

function localDayNumber(date: Date): number {
  return Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / DAY_MS;
}

function formatShortWeekday(date: Date): string {
  const locale = currentLocale();
  const value = localeFormatter('weekday', WEEKDAY_OPTIONS, locale).format(date);
  // German abbreviates weekdays with a trailing dot; English does not.
  if (locale !== 'de' || value.endsWith('.')) return value;
  return `${value}.`;
}

/** Compact list-row time: "14:05", "Yesterday", "Mon", "3 Aug", "3 Aug 2024". */
export function formatListTime(value: string): string {
  const date = new Date(value);
  const now = new Date();
  const diffDays = localDayNumber(now) - localDayNumber(date);
  if (diffDays === 0) return localeFormatter('time', TIME_OPTIONS).format(date);
  if (diffDays === 1) return t('common.yesterday');
  if (diffDays < 7) return formatShortWeekday(date);
  if (date.getFullYear() === now.getFullYear()) {
    return localeFormatter('date', DATE_OPTIONS).format(date);
  }
  return localeFormatter('dateWithYear', DATE_WITH_YEAR_OPTIONS).format(date);
}

/** Full timestamp used in expanded message headers. */
export function formatMessageTimestamp(value: string): string {
  return localeFormatter('messageTimestamp', MESSAGE_TIMESTAMP_OPTIONS).format(new Date(value));
}
