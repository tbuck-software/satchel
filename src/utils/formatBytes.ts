import { currentLocale } from '@/i18n/runtime';

/*
 * Decimal units, as Finder and Intl name them: 1 kB is 1000 bytes. The five
 * formatters this replaced mixed 10^6 and 2^20, so composer and reader
 * disagreed about the same file.
 */
const UNITS = [
  { unit: 'byte', size: 1, digits: 0 },
  { unit: 'kilobyte', size: 1e3, digits: 0 },
  { unit: 'megabyte', size: 1e6, digits: 1 },
  { unit: 'gigabyte', size: 1e9, digits: 1 },
] as const;

const formatters = new Map<string, Intl.NumberFormat>();

function formatter(locale: string, unit: (typeof UNITS)[number]): Intl.NumberFormat {
  const key = `${locale}:${unit.unit}`;
  let cached = formatters.get(key);
  if (!cached) {
    cached = new Intl.NumberFormat(locale, {
      style: 'unit',
      unit: unit.unit,
      // "512 bytes" reads better than "512 byte" or "512B".
      unitDisplay: unit.unit === 'byte' ? 'long' : 'short',
      maximumFractionDigits: unit.digits,
    });
    formatters.set(key, cached);
  }
  return cached;
}

/** `184 kB`, `1.2 MB` in English, `1,2 MB` in German — in the app locale. */
export function formatBytes(bytes: number, locale: string = currentLocale()): string {
  const value = Math.max(0, bytes);
  let index = UNITS.length - 1;
  while (index > 0 && value < UNITS[index]!.size) index -= 1;
  let unit = UNITS[index]!;
  // 999,950 bytes would round to "1,000 kB"; that is "1 MB".
  const next = UNITS[index + 1];
  if (next && Number((value / unit.size).toFixed(unit.digits)) >= 1000) unit = next;
  return formatter(locale, unit).format(value / unit.size);
}
