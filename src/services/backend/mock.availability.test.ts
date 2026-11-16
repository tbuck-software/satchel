import { beforeEach, expect, it, vi } from 'vitest';
import type { BackendFreeBusyReport, BackendFreeSlotsReport } from './availability';

let sendCommand: typeof import('./mock')['sendCommand'];

beforeEach(async () => {
  vi.resetModules();
  ({ sendCommand } = await import('./mock'));
}, 30_000);

/** A Monday well clear of the fixtures' own events. */
function monday(): Date {
  const day = new Date();
  day.setHours(0, 0, 0, 0);
  day.setDate(day.getDate() + ((8 - day.getDay()) % 7 || 7) + 14);
  return day;
}

it('answers for people inside the organisation and leaves outsiders unknown', async () => {
  const start = monday();
  const end = new Date(start.getTime() + 86_400_000);
  const report = await sendCommand<BackendFreeBusyReport>('query_free_busy', { input: {
    account_id: 'acc-work', attendees: ['Lena.Vogel@app.io', 'marco@studio-rossi.example', 'tom@satchel.dev'],
    starts_at: start.toISOString(), ends_at: end.toISOString(), time_zone: 'Europe/Berlin', excluded_calendars: [],
  } });
  // The owner's own address is the "you" row, not an attendee.
  expect(report.attendees.map((entry) => [entry.address, entry.known])).toEqual([
    ['lena.vogel@app.io', true],
    ['marco@studio-rossi.example', false],
  ]);
  const lena = report.attendees[0]!.busy;
  expect(lena.length).toBeGreaterThanOrEqual(2);
  for (const block of lena) {
    expect(Date.parse(block.ends_at)).toBeGreaterThan(Date.parse(block.starts_at));
    expect(Date.parse(block.starts_at)).toBeGreaterThanOrEqual(start.getTime());
  }
  // The same person is busy at the same times on every ask.
  const again = await sendCommand<BackendFreeBusyReport>('query_free_busy', { input: {
    account_id: 'acc-work', attendees: ['lena.vogel@app.io'], starts_at: start.toISOString(), ends_at: end.toISOString(),
    time_zone: 'Europe/Berlin', excluded_calendars: [],
  } });
  expect(again.attendees[0]!.busy).toEqual(lena);
});

it('finds free windows inside working hours around everyone known', async () => {
  const start = monday();
  start.setHours(6);
  const report = await sendCommand<BackendFreeSlotsReport>('find_free_slots', { input: {
    account_id: 'acc-work', attendees: ['lena.vogel@app.io', 'marco@studio-rossi.example'], from: start.toISOString(),
    working_days: 3, duration_minutes: 30, working_hours: { start_minute: 540, end_minute: 1020, weekdays: [1, 2, 3, 4, 5] },
    time_zone: 'Europe/Berlin', excluded_calendars: [],
  } });
  expect(report.unknown).toEqual(['marco@studio-rossi.example']);
  expect(report.days).toHaveLength(3);
  const busy = (await sendCommand<BackendFreeBusyReport>('query_free_busy', { input: {
    account_id: 'acc-work', attendees: ['lena.vogel@app.io'], starts_at: start.toISOString(),
    ends_at: new Date(start.getTime() + 7 * 86_400_000).toISOString(), time_zone: 'Europe/Berlin', excluded_calendars: [],
  } })).attendees[0]!.busy;
  for (const window of report.days.flatMap((day) => day.windows)) {
    const from = Date.parse(window.starts_at);
    const to = Date.parse(window.ends_at);
    expect(to - from).toBeGreaterThanOrEqual(30 * 60_000);
    expect(new Date(from).getHours() * 60 + new Date(from).getMinutes()).toBeGreaterThanOrEqual(540);
    expect(busy.some((block) => Date.parse(block.starts_at) < to && Date.parse(block.ends_at) > from)).toBe(false);
  }
});

it('refuses working hours that end before they start', async () => {
  await expect(sendCommand('find_free_slots', { input: {
    account_id: null, attendees: [], from: new Date().toISOString(), working_days: 3, duration_minutes: 30,
    working_hours: { start_minute: 600, end_minute: 540, weekdays: [1] }, time_zone: 'UTC', excluded_calendars: [],
  } })).rejects.toThrow(/availabilityWorkingHoursInvalid|working hours/i);
});

it('counts a whole-day event without TRANSP as busy and leaves the edited event out', async () => {
  const day = monday();
  day.setDate(day.getDate() + 1);
  const next = new Date(day);
  next.setDate(next.getDate() + 1);
  const ymd = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
  const create = (input: Record<string, unknown>) => sendCommand<{ id: string }>('create_calendar_event', { input: {
    id: null, expected_updated_at: null, account_id: 'acc-work', description: null, location: null,
    attendees: [], send_invites: false, ...input,
  } });
  await create({ title: 'Vacation', starts_at: ymd(day), ends_at: ymd(next), all_day: true });
  const meetingStart = new Date(next);
  meetingStart.setHours(10);
  const meetingEnd = new Date(next);
  meetingEnd.setHours(11);
  const meeting = await create({ title: 'Review', starts_at: meetingStart.toISOString(), ends_at: meetingEnd.toISOString(), all_day: false });

  const ask = (editing: unknown) => sendCommand<BackendFreeBusyReport>('query_free_busy', { input: {
    account_id: 'acc-work', attendees: [], starts_at: day.toISOString(),
    ends_at: new Date(next.getTime() + 86_400_000).toISOString(), time_zone: 'Europe/Berlin', excluded_calendars: [], editing,
  } });
  const plain = await ask(null);
  expect(plain.own.some((block) => Date.parse(block.starts_at) === day.getTime() && Date.parse(block.ends_at) === next.getTime())).toBe(true);
  expect(plain.own.some((block) => Date.parse(block.starts_at) === meetingStart.getTime())).toBe(true);

  const editing = await ask({ event_id: meeting.id, starts_at: meetingStart.toISOString(), ends_at: meetingEnd.toISOString() });
  expect(editing.own.some((block) => Date.parse(block.starts_at) === meetingStart.getTime())).toBe(false);
});
