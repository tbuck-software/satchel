import { readdirSync, readFileSync } from 'node:fs';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/*
 * Dates and numbers follow the language chosen in the app, not the one the
 * operating system runs in. Without a locale argument the Intl helpers take
 * the system's, so a German UI on an English Mac printed English dates.
 */

const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));
const SOURCE_ROOT = join(REPOSITORY_ROOT, 'src');

/** `toLocaleString()`, `toLocaleDateString(undefined, …)`, `new Intl.NumberFormat()` and kin. */
export const SYSTEM_LOCALE_CALL =
  /\.toLocale(?:Date|Time)?String\(\s*(?:\)|undefined\b)|new Intl\.\w+\(\s*(?:\)|undefined\b)/g;

function sourceFiles(dir = SOURCE_ROOT): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(full);
    if (!['.ts', '.vue'].includes(extname(entry.name)) || entry.name.includes('.test.')) return [];
    return [full];
  });
}

describe('formatting uses the app locale', () => {
  it('never falls back to the system locale', () => {
    const hits = sourceFiles().flatMap((file) => {
      const text = readFileSync(file, 'utf8');
      return [...text.matchAll(SYSTEM_LOCALE_CALL)].map((match) => {
        const line = text.slice(0, match.index).split('\n').length;
        return `${relative(REPOSITORY_ROOT, file)}:${line} ${match[0]}`;
      });
    });

    expect(hits).toEqual([]);
  });

  it('recognises the calls it is meant to catch', () => {
    const sample = [
      'new Date(value).toLocaleString();',
      'date.toLocaleDateString(undefined, { month: "long" });',
      'new Intl.NumberFormat().format(1);',
      'date.toLocaleString(locale.value);',
      'new Intl.DateTimeFormat(locale, options);',
    ].join('\n');

    expect([...sample.matchAll(SYSTEM_LOCALE_CALL)].map((match) => match[0])).toEqual([
      '.toLocaleString()',
      '.toLocaleDateString(undefined',
      'new Intl.NumberFormat()',
    ]);
  });
});
