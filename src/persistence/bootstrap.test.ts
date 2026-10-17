import { afterEach, describe, expect, it, vi } from 'vitest';
import { bootstrapPersistence } from './bootstrap';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from './index';

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

describe('bootstrap persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
  });

  it('reads timestamps with the existing Number conversion semantics', () => {
    const storage = memoryStorage({
      'satchel.signatures.lastDiscoveryAt': '1725105600000',
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(bootstrapPersistence.lastSignatureDiscoveryAt.read()).toBe(1725105600000);

    storage.values.set('satchel.signatures.lastDiscoveryAt', '');
    expect(bootstrapPersistence.lastSignatureDiscoveryAt.read()).toBe(0);

    storage.values.set('satchel.signatures.lastDiscoveryAt', 'Infinity');
    expect(bootstrapPersistence.lastSignatureDiscoveryAt.read()).toBe(Infinity);

    storage.values.set('satchel.signatures.lastDiscoveryAt', 'not-a-number');
    expect(bootstrapPersistence.lastSignatureDiscoveryAt.read()).toBeNaN();
  });

  it('falls back to zero when no timestamp is stored', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(bootstrapPersistence.lastSignatureDiscoveryAt.read()).toBe(0);
  });

  it('writes timestamps with their legacy string encoding', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    bootstrapPersistence.lastSignatureDiscoveryAt.write(1725105600000);

    expect(storage.values.get('satchel.signatures.lastDiscoveryAt')).toBe('1725105600000');
  });

  it('falls back and keeps writes safe when storage is unavailable', () => {
    const setItem = vi.fn(() => {
      throw new Error('set denied');
    });
    const storage: StorageLike = {
      getItem: () => {
        throw new Error('get denied');
      },
      setItem,
      removeItem: () => undefined,
    };
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(bootstrapPersistence.lastSignatureDiscoveryAt.read()).toBe(0);
    expect(() => bootstrapPersistence.lastSignatureDiscoveryAt.write(1725105600000))
      .not.toThrow();
    expect(setItem).toHaveBeenCalledOnce();
  });
});
