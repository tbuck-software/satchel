import type { FreeBusyInput } from './generated/app/commands/availability/FreeBusyInput';
import type { FreeSlotsInput } from './generated/app/commands/availability/FreeSlotsInput';
import type { FreeBusyReport as WireFreeBusyReport } from './generated/core/calendar/free_busy/FreeBusyReport';
import type { FreeSlotsReport as WireFreeSlotsReport } from './generated/core/calendar/free_busy/FreeSlotsReport';
import type { AttendeeFreeBusy as WireAttendeeFreeBusy } from './generated/core/calendar/free_busy/AttendeeFreeBusy';
import type { BusyBlock as WireBusyBlock } from './generated/core/calendar/free_busy/BusyBlock';
import { invoke } from './core';

/** One busy interval, RFC 3339 UTC. */
export type BackendBusyBlock = WireBusyBlock;
/** An attendee's busy time, or `known: false` when nobody would say. */
export type BackendAttendeeFreeBusy = WireAttendeeFreeBusy;
export type BackendFreeBusyReport = WireFreeBusyReport;
/** Free windows per working day, read in `time_zone`. */
export type BackendFreeSlotsReport = WireFreeSlotsReport;
export type BackendFreeBusyInput = FreeBusyInput;
export type BackendFreeSlotsInput = FreeSlotsInput;

/**
 * Busy time of the account owner (from the local calendars) and of each
 * attendee (asked of the event's account) over a range. Attendees nobody
 * answers for come back unknown; that is not an error.
 */
export async function backendQueryFreeBusy(input: FreeBusyInput): Promise<BackendFreeBusyReport> {
  return invoke<BackendFreeBusyReport>('query_free_busy', { input });
}

/** Free windows within working hours for the owner and every known attendee. */
export async function backendFindFreeSlots(input: FreeSlotsInput): Promise<BackendFreeSlotsReport> {
  return invoke<BackendFreeSlotsReport>('find_free_slots', { input });
}
