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
  expect(events.filter((event) => event.uid === 'standup-series@outlook.example')).toHaveLength(2);
  const sources = await sendCommand<BackendCalendarSource[]>('list_calendar_sources', { accountId: null });
  expect(sources).toContainEqual({ account_id: 'acc-outlook', source: 'outlook', remote_id: 'outlook-calendar-acc-outlook', name: 'Calendar' });
});

it('refuses an Outlook contacts grant like the backend does', async () => {
  await expect(sendCommand('start_oauth_flow', {
    input: { account_id: 'acc-outlook', provider: 'outlook', purpose: 'contacts' },
  })).rejects.toThrow('oauthConfigurationInvalid');
});
