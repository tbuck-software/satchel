import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from '@/persistence';
import { useCalendarUiStore } from './calendarUi';

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

describe('calendar UI persistence', () => {
  let restoreProvider: (() => void) | undefined;

  beforeEach(() => {
    setActivePinia(createPinia());
  });

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
  });

  it('hydrates and persists the view and hidden sources', () => {
    const storage = memoryStorage({
      'satchel:calendar-view': 'week',
      'satchel:calendar-hidden-sources': '["work"]',
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);
    const calendarUi = useCalendarUiStore();

    expect(calendarUi.viewMode).toBe('week');
    expect(calendarUi.hiddenSources).toEqual(['work']);

    calendarUi.setViewMode('day');
    calendarUi.toggleSource('personal');

    expect(storage.values.get('satchel:calendar-view')).toBe('day');
    expect(storage.values.get('satchel:calendar-hidden-sources')).toBe(
      '["work","personal"]',
    );
  });

  it('keeps state updates safe when storage is unavailable', () => {
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
    const calendarUi = useCalendarUiStore();

    expect(calendarUi.viewMode).toBe('month');
    expect(calendarUi.hiddenSources).toEqual([]);
    expect(() => calendarUi.setViewMode('week')).not.toThrow();
    expect(() => calendarUi.toggleSource('work')).not.toThrow();
    expect(calendarUi.viewMode).toBe('week');
    expect(calendarUi.hiddenSources).toEqual(['work']);
    expect(setItem).toHaveBeenCalledTimes(2);
  });
});
