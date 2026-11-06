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

  it('keeps working hours only when they make sense, else nine to five on weekdays', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(calendarPersistence.workingHours.read()).toEqual({ startMinute: 540, endMinute: 1020, weekdays: [1, 2, 3, 4, 5] });

    calendarPersistence.workingHours.write({ startMinute: 480, endMinute: 960, weekdays: [2, 4] });
    expect(calendarPersistence.workingHours.read()).toEqual({ startMinute: 480, endMinute: 960, weekdays: [2, 4] });

    storage.values.set('satchel:calendar-working-hours', JSON.stringify({ startMinute: 900, endMinute: 600, weekdays: [1] }));
    expect(calendarPersistence.workingHours.read().startMinute).toBe(540);
    storage.values.set('satchel:calendar-working-hours', JSON.stringify({ startMinute: 480, endMinute: 960, weekdays: [9, 0] }));
    expect(calendarPersistence.workingHours.read().weekdays).toEqual([1, 2, 3, 4, 5]);
  });

  it('remembers the calendars that do not count for availability', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    calendarPersistence.availabilityExcludedKeys.write(['google:family@group.calendar.google.com']);
    expect(storage.values.get('satchel:calendar-availability-excluded')).toBe('["google:family@group.calendar.google.com"]');
    expect(calendarPersistence.availabilityExcludedKeys.read()).toEqual(['google:family@group.calendar.google.com']);
  });
});
