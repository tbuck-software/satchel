import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from './index';
import { hasStoredTheme, readTheme, THEME_STORAGE_KEY, writeTheme } from './theme';

function memoryStorage(initial: Record<string, string> = {}): StorageLike {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
    removeItem: (key) => {
      values.delete(key);
    },
  };
}

function stubSystemPreference(light: boolean) {
  vi.stubGlobal('window', {
    matchMedia: vi.fn(() => ({ matches: light })),
  });
}

describe('theme persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
    vi.unstubAllGlobals();
  });

  it.each(['dark', 'light'] as const)('uses a valid stored %s theme', (theme) => {
    const storage = memoryStorage({ [THEME_STORAGE_KEY]: theme });
    restoreProvider = setPersistenceStorageProvider(() => storage);
    stubSystemPreference(theme === 'dark');

    expect(readTheme()).toBe(theme);
    expect(hasStoredTheme()).toBe(true);
  });

  it.each([
    { stored: undefined, light: true, expected: 'light' },
    { stored: undefined, light: false, expected: 'dark' },
    { stored: 'sepia', light: true, expected: 'light' },
  ] as const)('resolves $expected when storage is $stored', ({ stored, light, expected }) => {
    const storage = memoryStorage(
      stored === undefined ? {} : { [THEME_STORAGE_KEY]: stored },
    );
    restoreProvider = setPersistenceStorageProvider(() => storage);
    stubSystemPreference(light);

    expect(readTheme()).toBe(expected);
    expect(hasStoredTheme()).toBe(stored !== undefined);
  });

  it('writes the exact theme value to the stable storage key', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);
    stubSystemPreference(false);

    writeTheme('light');

    expect(storage.getItem(THEME_STORAGE_KEY)).toBe('light');
    expect(readTheme()).toBe('light');
  });

  it('falls back to the dark default without a window', () => {
    restoreProvider = setPersistenceStorageProvider(() => undefined);
    vi.stubGlobal('window', undefined);

    expect(readTheme()).toBe('dark');
    expect(hasStoredTheme()).toBe(false);
    expect(() => writeTheme('light')).not.toThrow();
  });

  it('falls back to the system preference when storage operations throw', () => {
    const throwingStorage: StorageLike = {
      getItem: () => {
        throw new Error('get denied');
      },
      setItem: () => {
        throw new Error('set denied');
      },
      removeItem: () => {
        throw new Error('remove denied');
      },
    };
    restoreProvider = setPersistenceStorageProvider(() => throwingStorage);
    stubSystemPreference(true);

    expect(readTheme()).toBe('light');
    expect(hasStoredTheme()).toBe(false);
    expect(() => writeTheme('dark')).not.toThrow();
  });
});
