import { afterEach, describe, expect, it } from 'vitest';
import {
  LOCALE_STORAGE_KEY,
  readStoredLocalePreference,
  writeStoredLocalePreference,
} from './locale';
import { setPersistenceStorageProvider, type StorageLike } from '@/persistence';

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

describe('locale preference persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
  });

  it('preserves the locale storage key and system fallback', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(LOCALE_STORAGE_KEY).toBe('satchel.locale');
    expect(readStoredLocalePreference()).toBe('system');
  });

  it.each(['en', 'de', 'system'] as const)('round-trips the %s preference', (preference) => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    writeStoredLocalePreference(preference);

    expect(storage.getItem(LOCALE_STORAGE_KEY)).toBe(preference);
    expect(readStoredLocalePreference()).toBe(preference);
  });

  it('uses system for an invalid stored preference', () => {
    const storage = memoryStorage({ [LOCALE_STORAGE_KEY]: 'fr' });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(readStoredLocalePreference()).toBe('system');
  });
});
