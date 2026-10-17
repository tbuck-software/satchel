import { afterEach, describe, expect, it } from 'vitest';
import { calendarPersistence } from './calendar';
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

describe('calendar persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
  });

  it('reads supported view modes and falls back to month', () => {
    const storage = memoryStorage({ 'satchel:calendar-view': 'week' });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(calendarPersistence.viewMode.read()).toBe('week');

    storage.values.set('satchel:calendar-view', 'day');
    expect(calendarPersistence.viewMode.read()).toBe('day');

    storage.values.set('satchel:calendar-view', 'agenda');
    expect(calendarPersistence.viewMode.read()).toBe('month');
  });

  it('keeps string source keys in their stored order', () => {
    const storage = memoryStorage({
      'satchel:calendar-hidden-sources': JSON.stringify(['b', 42, '', 'a', 'b']),
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(calendarPersistence.hiddenSourceKeys.read()).toEqual(['b', '', 'a', 'b']);
  });

  it('falls back for malformed and non-array hidden-source values', () => {
    const storage = memoryStorage({ 'satchel:calendar-hidden-sources': '{broken' });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(calendarPersistence.hiddenSourceKeys.read()).toEqual([]);

    storage.values.set('satchel:calendar-hidden-sources', JSON.stringify({ source: 'work' }));
    expect(calendarPersistence.hiddenSourceKeys.read()).toEqual([]);
  });

  it('writes both values with their legacy encodings', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    calendarPersistence.viewMode.write('day');
    calendarPersistence.hiddenSourceKeys.write(['work', 'personal']);

    expect(storage.values.get('satchel:calendar-view')).toBe('day');
    expect(storage.values.get('satchel:calendar-hidden-sources')).toBe(
      '["work","personal"]',
    );
  });
});
