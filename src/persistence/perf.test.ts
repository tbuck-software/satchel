import { afterEach, describe, expect, it } from 'vitest';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from './index';
import { perfPersistence } from './perf';

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

describe('performance diagnostics persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
  });

  it('enables diagnostics only for the exact legacy value', () => {
    const storage = memoryStorage({ 'satchel:perf': '1' });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(perfPersistence.enabled.read()).toBe(true);

    storage.values.set('satchel:perf', 'true');
    expect(perfPersistence.enabled.read()).toBe(false);

    storage.values.set('satchel:perf', '0');
    expect(perfPersistence.enabled.read()).toBe(false);
  });

  it('falls back when the flag is missing or storage rejects the read', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(perfPersistence.enabled.read()).toBe(false);

    restoreProvider();
    restoreProvider = setPersistenceStorageProvider(() => ({
      getItem: () => {
        throw new Error('get denied');
      },
      setItem: () => undefined,
      removeItem: () => undefined,
    }));
    expect(perfPersistence.enabled.read()).toBe(false);
  });
});
