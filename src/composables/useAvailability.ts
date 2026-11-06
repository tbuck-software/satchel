import {
  backendFindFreeSlots,
  backendQueryFreeBusy,
  type BackendFreeBusyInput,
  type BackendFreeBusyReport,
  type BackendFreeSlotsInput,
  type BackendFreeSlotsReport,
} from '@/services/backend/availability';
import { backendCommandsAvailable } from '@/services/backend/transport';
import { useCalendarUiStore } from '@/stores/calendarUi';
import { localTimeZone, workingHoursInput } from '@/utils/workingHours';

export type { BackendBusyBlock, BackendFreeBusyReport, BackendFreeSlotsReport } from '@/services/backend/availability';

/** Who is asked about, and the event being edited, if any. */
type Scope = Pick<BackendFreeBusyInput, 'account_id' | 'attendees'> & {
  editing?: { id: string; start: Date; end: Date } | null;
};

function wireScope(scope: Scope) {
  return {
    account_id: scope.account_id,
    attendees: scope.attendees,
    editing: scope.editing
      ? { event_id: scope.editing.id, starts_at: scope.editing.start.toISOString(), ends_at: scope.editing.end.toISOString() }
      : null,
  };
}

/**
 * Free/busy and free slots with the settings that shape them filled in:
 * this device's time zone, the working hours, and the calendars switched off
 * for availability. Components pass only who and when.
 */
export function useAvailability() {
  const calendarUi = useCalendarUiStore();

  function settings() {
    return {
      time_zone: localTimeZone(),
      excluded_calendars: [...calendarUi.availabilityExcluded],
    };
  }

  return {
    commandsAvailable: backendCommandsAvailable,

    freeBusy(scope: Scope, range: { start: Date; end: Date }): Promise<BackendFreeBusyReport> {
      return backendQueryFreeBusy({
        ...wireScope(scope),
        starts_at: range.start.toISOString(),
        ends_at: range.end.toISOString(),
        ...settings(),
      });
    },

    freeSlots(
      scope: Scope,
      search: Pick<BackendFreeSlotsInput, 'working_days' | 'duration_minutes'> & { from: Date },
    ): Promise<BackendFreeSlotsReport> {
      return backendFindFreeSlots({
        ...wireScope(scope),
        from: search.from.toISOString(),
        working_days: search.working_days,
        duration_minutes: search.duration_minutes,
        working_hours: workingHoursInput(calendarUi.workingHours),
        ...settings(),
      });
    },
  };
}
