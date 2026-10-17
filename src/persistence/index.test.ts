import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createPersistedSlot,
  persistence,
  setPersistenceStorageProvider,
  type StorageLike,
} from './index';

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

describe('Persistence', () => {
  let restoreProvider: (() => void) | undefined;

  beforeEach(() => {
    restoreProvider = undefined;
  });

  afterEach(() => {
    restoreProvider?.();
    vi.unstubAllGlobals();
  });

  it('reads, writes, and removes typed values through a slot', () => {
    const storage = memoryStorage({ count: '41' });
    restoreProvider = setPersistenceStorageProvider(() => storage);
    const slot = createPersistedSlot<number>('count', {
      fallback: () => 0,
      decode: (raw) => Number(raw),
      encode: (value) => String(value),
    });

    expect(persistence.read(slot)).toBe(41);
    expect(persistence.write(slot, 42)).toBe(true);
    expect(persistence.read(slot)).toBe(42);
    expect(persistence.remove(slot)).toBe(true);
    expect(persistence.read(slot)).toBe(0);
  });

  it('resolves the provider lazily and supports swapping providers', () => {
    const first = memoryStorage({ value: 'first' });
    const second = memoryStorage({ value: 'second' });
    const provider = vi.fn(() => first);
    restoreProvider = setPersistenceStorageProvider(provider);
    const slot = createPersistedSlot<string>('value', {
      fallback: () => 'fallback',
      decode: (raw) => raw,
      encode: (value) => value,
    });

    expect(provider).not.toHaveBeenCalled();
    expect(persistence.read(slot)).toBe('first');
    expect(provider).toHaveBeenCalledTimes(1);

    const restoreSecond = setPersistenceStorageProvider(() => second);
    expect(persistence.read(slot)).toBe('second');
    restoreSecond();
    expect(persistence.read(slot)).toBe('first');
  });

  it('falls back when no storage is available', () => {
    restoreProvider = setPersistenceStorageProvider(() => undefined);
    const decode = vi.fn((raw: string) => raw);
    const slot = createPersistedSlot('missing', {
      fallback: () => 'fallback',
      decode,
      encode: (value) => value,
    });

    expect(persistence.read(slot)).toBe('fallback');
    expect(persistence.write(slot, 'value')).toBe(false);
    expect(persistence.remove(slot)).toBe(false);
    expect(decode).not.toHaveBeenCalled();
  });

  it('falls back when the browser window or its storage getter is unavailable', () => {
    const slot = createPersistedSlot('key', {
      fallback: () => 'fallback',
      decode: (raw) => raw,
      encode: (value) => value,
    });
    vi.stubGlobal('window', undefined);
    expect(persistence.read(slot)).toBe('fallback');
    expect(persistence.write(slot, 'value')).toBe(false);
    expect(persistence.remove(slot)).toBe(false);

    vi.stubGlobal('window', {
      get localStorage(): never {
        throw new Error('localStorage getter denied');
      },
    });
    expect(persistence.read(slot)).toBe('fallback');
  });

  it('falls back when the storage provider throws', () => {
    restoreProvider = setPersistenceStorageProvider(() => {
      throw new Error('storage getter denied');
    });
    const slot = createPersistedSlot('key', {
      fallback: () => 'fallback',
      decode: (raw) => raw,
      encode: (value) => value,
    });

    expect(persistence.read(slot)).toBe('fallback');
    expect(persistence.write(slot, 'value')).toBe(false);
    expect(persistence.remove(slot)).toBe(false);
  });

  it('falls back when storage operations throw', () => {
    const storage: StorageLike = {
      getItem: () => {
        throw new Error('get failed');
      },
      setItem: () => {
        throw new Error('set failed');
      },
      removeItem: () => {
        throw new Error('remove failed');
      },
    };
    restoreProvider = setPersistenceStorageProvider(() => storage);
    const slot = createPersistedSlot('key', {
      fallback: () => 'fallback',
      decode: (raw) => raw,
      encode: (value) => value,
    });

    expect(persistence.read(slot)).toBe('fallback');
    expect(persistence.write(slot, 'value')).toBe(false);
    expect(persistence.remove(slot)).toBe(false);
  });

  it('falls back when decoding or encoding throws', () => {
    const storage = memoryStorage({ key: 'raw' });
    restoreProvider = setPersistenceStorageProvider(() => storage);
    const decodingSlot = createPersistedSlot('key', {
      fallback: () => 'fallback',
      decode: () => {
        throw new Error('decode failed');
      },
      encode: (value: string) => value,
    });
    const encodingSlot = createPersistedSlot('key', {
      fallback: () => 'fallback',
      decode: (raw) => raw,
      encode: () => {
        throw new Error('encode failed');
      },
    });

    expect(persistence.read(decodingSlot)).toBe('fallback');
    expect(persistence.write(encodingSlot, 'value')).toBe(false);
    expect(persistence.read(encodingSlot)).toBe('raw');
  });

  it('creates a fresh fallback value for each read', () => {
    restoreProvider = setPersistenceStorageProvider(() => null);
    const slot = createPersistedSlot<string[]>('items', {
      fallback: () => [],
      decode: (raw) => JSON.parse(raw) as string[],
      encode: JSON.stringify,
    });

    const first = persistence.read(slot);
    first.push('changed');

    expect(persistence.read(slot)).toEqual([]);
  });

  it('preserves null when it is a valid decoded value', () => {
    restoreProvider = setPersistenceStorageProvider(() => memoryStorage({ value: 'null' }));
    const slot = createPersistedSlot<string | null>('value', {
      fallback: () => 'fallback',
      decode: (raw) => raw === 'null' ? null : raw,
      encode: (value) => value ?? 'null',
    });

    expect(persistence.read(slot)).toBeNull();
  });
});
