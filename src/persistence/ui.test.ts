import { afterEach, describe, expect, it } from 'vitest';
import { setPersistenceStorageProvider, type StorageLike } from './index';
import { uiPersistence } from './ui';

const UI_STORAGE_KEYS = {
  sidebarWidth: 'satchel.ui.sidebarWidth',
  threadListWidth: 'satchel.ui.threadListWidth',
  newSendersExpanded: 'satchel.ui.newSendersExpanded',
} as const;

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

describe('ui persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
  });

  it('keeps stable keys, defaults, and finite persisted values', () => {
    const storage = memoryStorage({
      [UI_STORAGE_KEYS.sidebarWidth]: '0',
      [UI_STORAGE_KEYS.threadListWidth]: '9999.5',
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(uiPersistence.sidebarWidth.read()).toBe(0);
    expect(uiPersistence.threadListWidth.read()).toBe(9999.5);

    storage.values.clear();
    expect(uiPersistence.sidebarWidth.read()).toBe(240);
    expect(uiPersistence.threadListWidth.read()).toBe(408);
  });

  it.each([
    ['', 0],
    ['  ', 0],
    ['-220.5', -220.5],
    ['Infinity', 240],
    ['NaN', 408],
    ['not-a-number', 240],
  ])('decodes legacy raw value %j with the existing Number semantics', (raw, expected) => {
    const key = raw === 'NaN' ? UI_STORAGE_KEYS.threadListWidth : UI_STORAGE_KEYS.sidebarWidth;
    const storage = memoryStorage({ [key]: raw });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(key === UI_STORAGE_KEYS.sidebarWidth
      ? uiPersistence.sidebarWidth.read()
      : uiPersistence.threadListWidth.read()).toBe(expected);
  });

  it('writes values unchanged through the stable keys', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    uiPersistence.sidebarWidth.write(220);
    uiPersistence.threadListWidth.write(760.5);

    expect(storage.values.get(UI_STORAGE_KEYS.sidebarWidth)).toBe('220');
    expect(storage.values.get(UI_STORAGE_KEYS.threadListWidth)).toBe('760.5');
  });

  it('reads the new-sender expansion as a strict boolean, collapsed by default', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(uiPersistence.newSendersExpanded.read()).toBe(false);
    uiPersistence.newSendersExpanded.write(true);
    expect(storage.values.get(UI_STORAGE_KEYS.newSendersExpanded)).toBe('true');
    expect(uiPersistence.newSendersExpanded.read()).toBe(true);

    storage.values.set(UI_STORAGE_KEYS.newSendersExpanded, 'yes');
    expect(uiPersistence.newSendersExpanded.read()).toBe(false);
  });

  it('tolerates unavailable and throwing storage', () => {
    restoreProvider = setPersistenceStorageProvider(() => undefined);

    expect(uiPersistence.sidebarWidth.read()).toBe(240);
    expect(uiPersistence.threadListWidth.read()).toBe(408);
    expect(() => uiPersistence.sidebarWidth.write(300)).not.toThrow();
    expect(() => uiPersistence.threadListWidth.write(500)).not.toThrow();

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
    const restoreThrowing = setPersistenceStorageProvider(() => throwingStorage);
    expect(uiPersistence.sidebarWidth.read()).toBe(240);
    expect(() => uiPersistence.sidebarWidth.write(300)).not.toThrow();
    restoreThrowing();
  });
});
