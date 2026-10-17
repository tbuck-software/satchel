import type { PluginMetrics as WirePluginMetrics } from './generated/app/plugins/runtime/PluginMetrics';
import type { OptionalWireFields } from './wireTypes';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type {
  BackendCalendarEvent,
  BackendCalendarInvite,
  BackendCalendarSource,
  BackendGoogleCalendarSyncSummary,
} from '@/types/calendar';

export type {
  BackendCalendarEvent,
  BackendCalendarInvite,
  BackendCalendarSource,
  BackendGoogleCalendarSyncSummary,
  BackendTrayCalendarEvent,
} from '@/types/calendar';

export type BackendPluginMetrics = OptionalWireFields<WirePluginMetrics, 'last_error' | 'last_called_at'>;


export async function backendListPluginMetrics(): Promise<BackendPluginMetrics[]> {
  return invoke<BackendPluginMetrics[]>('list_plugin_metrics');
}


export async function backendListInvites(accountId?: string): Promise<BackendCalendarInvite[]> {
  return invoke<BackendCalendarInvite[]>('list_invites', { accountId: accountId ?? null });
}


export async function backendListCalendarEvents(accountId?: string): Promise<BackendCalendarEvent[]> {
  return invoke<BackendCalendarEvent[]>('list_calendar_events', { accountId: accountId ?? null });
}


export async function backendListCalendarSources(accountId?: string): Promise<BackendCalendarSource[]> {
  return invoke<BackendCalendarSource[]>('list_calendar_sources', { accountId: accountId ?? null });
}


export async function backendCreateCalendarEvent(input: {
  id?: string | null;
  accountId: string;
  title: string;
  description?: string | null;
  location?: string | null;
  startsAt: string;
  endsAt: string;
  allDay?: boolean;
  attendees?: string[];
  sendInvites?: boolean;
}): Promise<BackendCalendarEvent> {
  return invoke<BackendCalendarEvent>('create_calendar_event', {
    input: {
      id: input.id ?? null,
      account_id: input.accountId,
      title: input.title,
      description: input.description ?? null,
      location: input.location ?? null,
      starts_at: input.startsAt,
      ends_at: input.endsAt,
      all_day: input.allDay ?? false,
      attendees: input.attendees ?? [],
      send_invites: input.sendInvites ?? false,
    },
  });
}


export async function backendDeleteCalendarEvent(id: string): Promise<void> {
  return invoke<void>('delete_calendar_event', { id });
}


export async function backendSyncGoogleCalendar(
  accountId?: string,
): Promise<BackendGoogleCalendarSyncSummary> {
  return invoke<BackendGoogleCalendarSyncSummary>('sync_google_calendar', {
    accountId: accountId ?? null,
  });
}


export async function backendRespondToInvite(input: {
  inviteId: string;
  response: 'accepted' | 'declined' | 'tentative';
  comment?: string | null;
}): Promise<void> {
  return invoke<void>('respond_to_invite', {
    input: {
      invite_id: input.inviteId,
      response: input.response,
      comment: input.comment ?? null,
    },
  });
}


export async function backendImportIcs(input: {
  raw: string;
  accountId?: string | null;
  sourceFilename?: string | null;
}): Promise<BackendCalendarInvite> {
  return invoke<BackendCalendarInvite>('import_ics', {
    input: {
      raw: input.raw,
      account_id: input.accountId ?? null,
      source_filename: input.sourceFilename ?? null,
    },
  });
}


export async function backendImportIcsPath(path: string): Promise<BackendCalendarInvite> {
  return invoke<BackendCalendarInvite>('import_ics_path', { path });
}

