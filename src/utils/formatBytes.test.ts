import { describe, expect, it } from 'vitest';
import { formatBytes } from './formatBytes';

// Intl separates number and unit with a no-break space in some locales.
const plain = (text: string) => text.replace(/ | /g, ' ');

describe('formatBytes', () => {
  it('uses the decimal separator of the app locale', () => {
    expect(plain(formatBytes(1_200_000, 'en'))).toBe('1.2 MB');
    expect(plain(formatBytes(1_200_000, 'de'))).toBe('1,2 MB');
  });

  it('picks one decimal unit per magnitude', () => {
    expect(plain(formatBytes(512, 'en'))).toBe('512 bytes');
    expect(plain(formatBytes(512, 'de'))).toBe('512 Byte');
    expect(plain(formatBytes(184_000, 'en'))).toBe('184 kB');
    expect(plain(formatBytes(25_000_000, 'en'))).toBe('25 MB');
    expect(plain(formatBytes(3_400_000_000, 'de'))).toBe('3,4 GB');
  });

  it('moves up a unit instead of printing 1,000 of the smaller one', () => {
    expect(plain(formatBytes(999_950, 'en'))).toBe('1 MB');
    expect(plain(formatBytes(999, 'en'))).toBe('999 bytes');
  });
});
