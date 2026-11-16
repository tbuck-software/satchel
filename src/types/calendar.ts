import type { CalendarInviteRecord as WireCalendarInviteRecord } from '../services/backend/generated/core/db/calendar/CalendarInviteRecord';
import type { CalendarEventRecord as WireCalendarEventRecord } from '../services/backend/generated/core/db/calendar/CalendarEventRecord';
import type { CalendarSourceRecord as WireCalendarSourceRecord } from '../services/backend/generated/core/db/calendar/CalendarSourceRecord';
import type { OptionalWireFields } from '../services/backend/wireTypes';
import type { GoogleCalendarSyncSummary as WireGoogleCalendarSyncSummary } from '../services/backend/generated/app/commands/calendar/GoogleCalendarSyncSummary';
import type { TrayCalendarEventDto as WireTrayCalendarEventDto } from '../services/backend/generated/app/commands/tray/TrayCalendarEventDto';
export type CalendarViewMode = 'month' | 'week' | 'day';

export type BackendCalendarInvite = OptionalWireFields<WireCalendarInviteRecord, 'message_id' | 'organizer_email' | 'organizer_name' | 'summary' | 'description' | 'location' | 'starts_at' | 'ends_at' | 'method' | 'cancelled' | 'recurrence_id' | 'recurring' | 'recurrence_unbounded' | 'sequence'>;

/** The provider flags are 0 for every non-Outlook row; fixtures may leave them out. */
export type BackendCalendarEvent = OptionalWireFields<WireCalendarEventRecord, 'description' | 'location' | 'collection_href' | 'read_only' | 'recurrence_instance' | 'rich_description'>;

export type BackendCalendarSource = WireCalendarSourceRecord;

export type BackendGoogleCalendarSyncSummary = WireGoogleCalendarSyncSummary;

export type BackendTrayCalendarEvent = OptionalWireFields<WireTrayCalendarEventDto, 'location'>;
