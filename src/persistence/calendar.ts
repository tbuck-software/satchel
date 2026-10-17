import { createPersistedSlot, persistence } from './index';
import type { CalendarViewMode } from '@/types/calendar';

const VIEW_STORAGE_KEY = 'satchel:calendar-view';
const HIDDEN_SOURCES_STORAGE_KEY = 'satchel:calendar-hidden-sources';

const viewModeSlot = createPersistedSlot<CalendarViewMode>(VIEW_STORAGE_KEY, {
  fallback: () => 'month',
  decode: (raw) => (raw === 'week' || raw === 'day' ? raw : 'month'),
  encode: (value) => value,
});

const hiddenSourceKeysSlot = createPersistedSlot<string[]>(HIDDEN_SOURCES_STORAGE_KEY, {
  fallback: () => [],
  decode: (raw) => {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((key): key is string => typeof key === 'string')
      : undefined;
  },
  encode: (value) => JSON.stringify(value),
});

export const calendarPersistence = {
  viewMode: {
    read: () => persistence.read(viewModeSlot),
    write: (value: CalendarViewMode) => {
      persistence.write(viewModeSlot, value);
    },
  },
  hiddenSourceKeys: {
    read: () => persistence.read(hiddenSourceKeysSlot),
    write: (value: string[]) => {
      persistence.write(hiddenSourceKeysSlot, value);
    },
  },
};
