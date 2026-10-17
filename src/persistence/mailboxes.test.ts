import { afterEach, describe, expect, it } from 'vitest';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from './index';
import { mailboxesPersistence } from './mailboxes';

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

describe('mailboxes persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
  });

  it('reads legacy favorite keys without changing their order or duplicates', () => {
    const storage = memoryStorage({
      'satchel.mailboxes.favoriteFolders': JSON.stringify([
        ' acc-2::Zeta ',
        'acc-1::beta',
        'acc-2::Zeta',
        'not-a-favorite',
        42,
        '',
      ]),
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(mailboxesPersistence.favoriteFolderKeys.read()).toEqual([
      'acc-2::Zeta',
      'acc-1::beta',
      'acc-2::Zeta',
    ]);
  });

  it('falls back for missing, malformed, and non-array values', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(mailboxesPersistence.favoriteFolderKeys.read()).toEqual([]);

    storage.values.set('satchel.mailboxes.favoriteFolders', '{broken');
    expect(mailboxesPersistence.favoriteFolderKeys.read()).toEqual([]);

    storage.values.set('satchel.mailboxes.favoriteFolders', JSON.stringify({ key: 'acc::folder' }));
    expect(mailboxesPersistence.favoriteFolderKeys.read()).toEqual([]);
  });

  it('normalizes writes to valid, unique, base-sorted keys', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    mailboxesPersistence.favoriteFolderKeys.write([
      'acc-2::zeta',
      'acc-1::Beta',
      'acc-2::zeta',
      'invalid',
      '',
      'acc-1::alpha',
    ]);

    expect(storage.values.get('satchel.mailboxes.favoriteFolders')).toBe(
      '["acc-1::alpha","acc-1::Beta","acc-2::zeta"]',
    );
  });

  it('does not throw when storage rejects writes', () => {
    const storage: StorageLike = {
      getItem: () => null,
      setItem: () => {
        throw new Error('set failed');
      },
      removeItem: () => undefined,
    };
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(() => mailboxesPersistence.favoriteFolderKeys.write(['acc::folder'])).not.toThrow();
  });
});
