// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from 'vitest';
import type { BackendCalendarEvent, BackendCalendarSource, BackendGoogleCalendarSyncSummary } from '@/types/calendar';

let sendCommand: typeof import('./mock')['sendCommand'];

beforeEach(async () => {
  vi.resetModules();
  window.history.replaceState({}, '', '/?mockOutlookCalendar=1');
  ({ sendCommand } = await import('./mock'));
}, 30_000);

const sync = () => sendCommand<BackendGoogleCalendarSyncSummary>('sync_outlook_calendar', { accountId: null });
const outlookEvents = async () => (await sendCommand<BackendCalendarEvent[]>('list_calendar_events', { accountId: null }))
  .filter((event) => event.source === 'outlook');

it('mirrors the Outlook calendar only after calendar access is connected', async () => {
  expect(await sync()).toEqual({ accounts_checked: 1, events_synced: 0, errors: [] });
  expect(await outlookEvents()).toEqual([]);

  const flow = await sendCommand<{ state: string }>('start_oauth_flow', {
    input: { account_id: 'acc-outlook', provider: 'outlook', purpose: 'calendar' },
  });
  const done = await sendCommand<{ result: { granted_scopes: string[] } }>('poll_oauth_flow', { input: { state: flow.state } });
  expect(done.result.granted_scopes).toContain('https://graph.microsoft.com/Calendars.ReadWrite');
  expect(done.result.granted_scopes).toContain('https://graph.microsoft.com/Mail.ReadWrite');

  expect((await sync()).events_synced).toBe(3);
  // A second pass changes nothing, like a delta with no changes.
  expect((await sync()).events_synced).toBe(3);
  const events = await outlookEvents();
  expect(events.map((event) => event.collection_href)).toEqual(Array(3).fill('outlook-calendar-acc-outlook'));
  // A series arrives as one row per occurrence, sharing its UID.
  const series = events.filter((event) => event.uid === 'standup-series@outlook.example');
  expect(series.map((event) => event.recurrence_instance)).toEqual([1, 1]);
  // A meeting the account only attends cannot be changed from here.
  expect(events.find((event) => event.title === 'Quarterly planning')).toMatchObject({ read_only: 1, rich_description: 1 });
  const sources = await sendCommand<BackendCalendarSource[]>('list_calendar_sources', { accountId: null });
  expect(sources).toContainEqual({ account_id: 'acc-outlook', source: 'outlook', remote_id: 'outlook-calendar-acc-outlook', name: 'Calendar' });
});

it('refuses an Outlook contacts grant like the backend does', async () => {
  await expect(sendCommand('start_oauth_flow', {
    input: { account_id: 'acc-outlook', provider: 'outlook', purpose: 'contacts' },
  })).rejects.toThrow('oauthConfigurationInvalid');
});

it('refuses to change or delete an event Outlook marks read-only, like the backend', async () => {
  const flow = await sendCommand<{ state: string }>('start_oauth_flow', {
    input: { account_id: 'acc-outlook', provider: 'outlook', purpose: 'calendar' },
  });
  await sendCommand('poll_oauth_flow', { input: { state: flow.state } });
  await sync();
  const events = await outlookEvents();
  const attended = events.find((event) => event.read_only === 1)!;
  const editable = events.find((event) => event.read_only === 0)!;
  const edit = (event: BackendCalendarEvent, title: string) => sendCommand<BackendCalendarEvent>('create_calendar_event', {
    input: { id: event.id, expected_updated_at: null, account_id: event.account_id, title, description: null,
      location: null, starts_at: event.starts_at, ends_at: event.ends_at, all_day: false, attendees: [], send_invites: false },
  });

  await expect(edit(attended, 'Renamed')).rejects.toThrow('calendarEventReadOnly');
  await expect(sendCommand('delete_calendar_event', { id: attended.id })).rejects.toThrow('calendarEventReadOnly');
  expect((await outlookEvents()).find((event) => event.id === attended.id)?.title).toBe(attended.title);

  expect((await edit(editable, 'Renamed')).title).toBe('Renamed');
  await sendCommand('delete_calendar_event', { id: editable.id });
  expect((await outlookEvents()).some((event) => event.id === editable.id)).toBe(false);
});

it('puts new events of a connected Outlook account in its calendar, guests only with invitations', async () => {
  const flow = await sendCommand<{ state: string }>('start_oauth_flow', {
    input: { account_id: 'acc-outlook', provider: 'outlook', purpose: 'calendar' },
  });
  await sendCommand('poll_oauth_flow', { input: { state: flow.state } });
  await sync();
  const create = (account_id: string, send_invites: boolean) => sendCommand<BackendCalendarEvent>('create_calendar_event', {
    input: { id: null, expected_updated_at: null, account_id, title: 'Lunch', description: null, location: null,
      starts_at: '2026-07-14T11:00:00Z', ends_at: '2026-07-14T12:00:00Z', all_day: false,
      attendees: ['bob@example.test'], send_invites },
  });

  expect(await create('acc-outlook', false)).toMatchObject({ source: 'outlook', attendees_json: '[]' });
  expect(await create('acc-outlook', true)).toMatchObject({ attendees_json: '["bob@example.test"]' });
  expect(await create('acc-work', false)).toMatchObject({ source: 'local', attendees_json: '["bob@example.test"]' });
});
