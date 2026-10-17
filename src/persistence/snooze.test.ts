import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from './index';
import { snoozePersistence } from './snooze';

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

describe('snooze persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
    vi.useRealTimers();
  });

  it('reads at most five future string dates in stored order', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-08-31T10:00:00Z'));
    const storage = memoryStorage({
      'satchel.waitingRecentDates': JSON.stringify([
        '2026-09-06T10:00:00Z',
        42,
        'invalid',
        '2026-08-30T10:00:00Z',
        '2026-09-05T10:00:00Z',
        '2026-09-04T10:00:00Z',
        '2026-09-03T10:00:00Z',
        '2026-09-02T10:00:00Z',
        '2026-09-01T10:00:00Z',
      ]),
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(snoozePersistence.recentManualDates.read()).toEqual([
      '2026-09-06T10:00:00Z',
      '2026-09-05T10:00:00Z',
      '2026-09-04T10:00:00Z',
      '2026-09-03T10:00:00Z',
      '2026-09-02T10:00:00Z',
    ]);
  });

  it('falls back for missing, malformed, and non-array values', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(snoozePersistence.recentManualDates.read()).toEqual([]);

    storage.values.set('satchel.waitingRecentDates', '{broken');
    expect(snoozePersistence.recentManualDates.read()).toEqual([]);

    storage.values.set('satchel.waitingRecentDates', JSON.stringify({ date: 'soon' }));
    expect(snoozePersistence.recentManualDates.read()).toEqual([]);
  });

  it('writes dates with their legacy JSON encoding', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    snoozePersistence.recentManualDates.write([
      '2026-09-02T10:00:00Z',
      '2026-09-01T10:00:00Z',
    ]);

    expect(storage.values.get('satchel.waitingRecentDates')).toBe(
      '["2026-09-02T10:00:00Z","2026-09-01T10:00:00Z"]',
    );
  });
});
