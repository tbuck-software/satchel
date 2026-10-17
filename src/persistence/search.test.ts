import { afterEach, describe, expect, it } from 'vitest';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from './index';
import { searchPersistence } from './search';

function memoryStorage(initial: Record<string, string> = {}): StorageLike & {
  values: Map<string, string>;
} {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}

describe('search persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
  });

  it('reads at most eight string entries without otherwise normalizing them', () => {
    const storage = memoryStorage({
      'satchel.search.recent': JSON.stringify([
        'one',
        2,
        '',
        'three',
        'four',
        'five',
        'six',
        'seven',
        'eight',
        'nine',
        'ten',
      ]),
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(searchPersistence.recentQueries.read()).toEqual([
      'one',
      '',
      'three',
      'four',
      'five',
      'six',
      'seven',
      'eight',
    ]);
  });

  it('falls back for missing, malformed, and non-array values', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(searchPersistence.recentQueries.read()).toEqual([]);

    storage.values.set('satchel.search.recent', '{broken');
    expect(searchPersistence.recentQueries.read()).toEqual([]);

    storage.values.set('satchel.search.recent', JSON.stringify({ query: 'one' }));
    expect(searchPersistence.recentQueries.read()).toEqual([]);
  });

  it('writes the list with its legacy JSON encoding', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    searchPersistence.recentQueries.write(['two', 'one']);

    expect(storage.values.get('satchel.search.recent')).toBe('["two","one"]');
  });
});
