import { MAX_RECENT_SNOOZE_DATES } from '@/config/historyLimits';
import { createPersistedSlot, persistence } from './index';

const RECENT_MANUAL_DATES_STORAGE_KEY = 'satchel.waitingRecentDates';

const recentManualDatesSlot = createPersistedSlot<string[]>(
  RECENT_MANUAL_DATES_STORAGE_KEY,
  {
    fallback: () => [],
    decode: (raw) => {
      const parsed: unknown = JSON.parse(raw);
      if (!Array.isArray(parsed)) return undefined;

      const now = Date.now();
      return parsed
        .filter((value): value is string => typeof value === 'string')
        .filter((value) => new Date(value).getTime() > now)
        .slice(0, MAX_RECENT_SNOOZE_DATES);
    },
    encode: (value) => JSON.stringify(value),
  },
);

export const snoozePersistence = {
  recentManualDates: {
    read: () => persistence.read(recentManualDatesSlot),
    write: (value: string[]) => {
      persistence.write(recentManualDatesSlot, value);
    },
  },
};
