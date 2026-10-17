import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from './index';
import { imagePolicyPersistence } from './imagePolicy';

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

describe('image policy persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
  });

  it('lowercases stored allow-list entries while preserving insertion order', () => {
    const storage = memoryStorage({
      'satchel:images-allowed-senders': '["Bob@Example.COM","ALICE@example.com"]',
      'satchel:images-allowed-domains': '["Example.COM","shop.example"]',
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(Array.from(imagePolicyPersistence.allowedSenders.read())).toEqual([
      'bob@example.com',
      'alice@example.com',
    ]);
    expect(Array.from(imagePolicyPersistence.allowedDomains.read())).toEqual([
      'example.com',
      'shop.example',
    ]);
  });

  it('falls back for malformed sets, non-arrays, and arrays with non-strings', () => {
    const storage = memoryStorage({
      'satchel:images-allowed-senders': '{broken',
      'satchel:images-allowed-domains': '["example.com",42]',
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(imagePolicyPersistence.allowedSenders.read()).toEqual(new Set());
    expect(imagePolicyPersistence.allowedDomains.read()).toEqual(new Set());

    storage.values.set('satchel:images-allowed-senders', '{"sender":"alice@example.com"}');
    expect(imagePolicyPersistence.allowedSenders.read()).toEqual(new Set());
  });

  it('prefers a valid mode and otherwise honors the legacy allow-all flag', () => {
    const storage = memoryStorage({
      'satchel:images-policy-mode': 'accepted',
      'satchel:images-allow-all': 'true',
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(imagePolicyPersistence.mode.read()).toBe('accepted');

    storage.values.set('satchel:images-policy-mode', 'invalid');
    expect(imagePolicyPersistence.mode.read()).toBe('all');

    storage.values.set('satchel:images-allow-all', 'false');
    expect(imagePolicyPersistence.mode.read()).toBe('allowlist');
  });

  it('writes sets as JSON and removes the legacy flag after a successful mode write', () => {
    const storage = memoryStorage({ 'satchel:images-allow-all': 'true' });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    imagePolicyPersistence.allowedSenders.write(new Set(['alice@example.com']));
    imagePolicyPersistence.mode.write('accepted');

    expect(storage.values.get('satchel:images-allowed-senders')).toBe('["alice@example.com"]');
    expect(storage.values.get('satchel:images-policy-mode')).toBe('accepted');
    expect(storage.values.has('satchel:images-allow-all')).toBe(false);
  });

  it('keeps the legacy flag when writing the replacement mode fails', () => {
    const removeItem = vi.fn();
    const storage: StorageLike = {
      getItem: () => 'true',
      setItem: () => {
        throw new Error('set denied');
      },
      removeItem,
    };
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(() => imagePolicyPersistence.mode.write('all')).not.toThrow();
    expect(removeItem).not.toHaveBeenCalled();
  });

  it('falls back and keeps mutations safe when storage is unavailable', () => {
    const setItem = vi.fn(() => {
      throw new Error('set denied');
    });
    const storage: StorageLike = {
      getItem: () => {
        throw new Error('get denied');
      },
      setItem,
      removeItem: () => {
        throw new Error('remove denied');
      },
    };
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(imagePolicyPersistence.allowedSenders.read()).toEqual(new Set());
    expect(imagePolicyPersistence.allowedDomains.read()).toEqual(new Set());
    expect(imagePolicyPersistence.mode.read()).toBe('allowlist');
    expect(() => imagePolicyPersistence.allowedSenders.write(new Set(['alice@example.com'])))
      .not.toThrow();
    expect(() => imagePolicyPersistence.mode.write('all')).not.toThrow();
    expect(setItem).toHaveBeenCalledTimes(2);
  });
});
