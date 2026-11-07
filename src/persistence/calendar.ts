import { createPersistedSlot, persistence } from './index';
import type { CalendarViewMode } from '@/types/calendar';
import { defaultWorkingHours, normalizeWorkingHours, type WorkingHours } from '@/utils/workingHours';

const VIEW_STORAGE_KEY = 'satchel:calendar-view';
const HIDDEN_SOURCES_STORAGE_KEY = 'satchel:calendar-hidden-sources';
const AVAILABILITY_EXCLUDED_STORAGE_KEY = 'satchel:calendar-availability-excluded';
const WORKING_HOURS_STORAGE_KEY = 'satchel:calendar-working-hours';

const viewModeSlot = createPersistedSlot<CalendarViewMode>(VIEW_STORAGE_KEY, {
  fallback: () => 'month',
  decode: (raw) => (raw === 'week' || raw === 'day' ? raw : 'month'),
  encode: (value) => value,
});

function stringListSlot(key: string) {
  return createPersistedSlot<string[]>(key, {
    fallback: () => [],
    decode: (raw) => {
      const parsed: unknown = JSON.parse(raw);
      return Array.isArray(parsed)
        ? parsed.filter((entry): entry is string => typeof entry === 'string')
        : undefined;
    },
    encode: (value) => JSON.stringify(value),
  });
}

const hiddenSourceKeysSlot = stringListSlot(HIDDEN_SOURCES_STORAGE_KEY);
/** Calendars switched off for free/busy: they no longer make the owner busy. */
const availabilityExcludedSlot = stringListSlot(AVAILABILITY_EXCLUDED_STORAGE_KEY);

const workingHoursSlot = createPersistedSlot<WorkingHours>(WORKING_HOURS_STORAGE_KEY, {
  fallback: defaultWorkingHours,
  decode: (raw) => normalizeWorkingHours(JSON.parse(raw)) ?? undefined,
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
  availabilityExcludedKeys: {
    read: () => persistence.read(availabilityExcludedSlot),
    write: (value: string[]) => {
      persistence.write(availabilityExcludedSlot, value);
    },
  },
  workingHours: {
    read: () => persistence.read(workingHoursSlot),
    write: (value: WorkingHours) => {
      persistence.write(workingHoursSlot, value);
    },
  },
};
