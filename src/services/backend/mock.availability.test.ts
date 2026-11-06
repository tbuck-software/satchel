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
