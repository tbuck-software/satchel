import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { calendarPersistence } from '@/persistence/calendar';
import type { CalendarViewMode } from '@/types/calendar';
import { startOfDay } from '@/utils/calendarGrid';
import type { WorkingHours } from '@/utils/workingHours';

export type { CalendarViewMode } from '@/types/calendar';

/**
 * View state shared between the calendar view and the calendar sidebar
 * (mini month, source filter). Lives in a store because the two live in
 * separate component trees.
 */
export const useCalendarUiStore = defineStore('calendarUi', () => {
  const anchorMs = ref(startOfDay(new Date()).getTime());
  const viewMode = ref<CalendarViewMode>(calendarPersistence.viewMode.read());
  const hiddenSources = ref<string[]>(calendarPersistence.hiddenSourceKeys.read());
  const availabilityExcluded = ref<string[]>(calendarPersistence.availabilityExcludedKeys.read());
  const workingHours = ref<WorkingHours>(calendarPersistence.workingHours.read());

  const anchor = computed(() => new Date(anchorMs.value));

  function setAnchor(date: Date) {
    anchorMs.value = startOfDay(date).getTime();
  }

  function setViewMode(mode: CalendarViewMode) {
    viewMode.value = mode;
    calendarPersistence.viewMode.write(mode);
  }

  function isSourceHidden(key: string): boolean {
    return hiddenSources.value.includes(key);
  }

  function toggleSource(key: string) {
    hiddenSources.value = isSourceHidden(key)
      ? hiddenSources.value.filter((entry) => entry !== key)
      : [...hiddenSources.value, key];
    calendarPersistence.hiddenSourceKeys.write(hiddenSources.value);
  }

  /** Whether a calendar makes the owner busy in free/busy and free slots. */
  function countsForAvailability(key: string): boolean {
    return !availabilityExcluded.value.includes(key);
  }

  function toggleAvailability(key: string) {
    availabilityExcluded.value = countsForAvailability(key)
      ? [...availabilityExcluded.value, key]
      : availabilityExcluded.value.filter((entry) => entry !== key);
    calendarPersistence.availabilityExcludedKeys.write(availabilityExcluded.value);
  }

  function setWorkingHours(next: WorkingHours) {
    workingHours.value = { ...next, weekdays: [...next.weekdays].sort((a, b) => a - b) };
    calendarPersistence.workingHours.write(workingHours.value);
  }

  return {
    anchor,
    viewMode,
    hiddenSources,
    availabilityExcluded,
    workingHours,
    setAnchor,
    setViewMode,
    isSourceHidden,
    toggleSource,
    countsForAvailability,
    toggleAvailability,
    setWorkingHours,
  };
});
