// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';

const backend = vi.hoisted(() => ({
  query: vi.fn(),
  find: vi.fn(),
}));

vi.mock('@/services/backend/availability', () => ({
  backendQueryFreeBusy: backend.query,
  backendFindFreeSlots: backend.find,
}));
vi.mock('@/services/backend/transport', () => ({ backendCommandsAvailable: () => true }));

import AvailabilityStrip from './AvailabilityStrip.vue';
import { useUiStore } from '@/stores/ui';
import { formatTime } from '@/utils/calendarGrid';

enableAutoUnmount(afterEach);
beforeEach(() => {
  setActivePinia(createPinia());
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
  backend.query.mockReset();
  backend.find.mockReset();
  backend.query.mockResolvedValue({
    own: [{ starts_at: new Date(2026, 10, 9, 9).toISOString(), ends_at: new Date(2026, 10, 9, 10).toISOString(), tentative: false }],
    attendees: [
      { address: 'lena@example.com', known: true, busy: [
        { starts_at: new Date(2026, 10, 9, 10, 30).toISOString(), ends_at: new Date(2026, 10, 9, 11, 30).toISOString(), tentative: false },
      ] },
      { address: 'outside@elsewhere.test', known: false, busy: [] },
    ],
  });
});
afterEach(() => vi.useRealTimers());

async function mountStrip(start = new Date(2026, 10, 9, 10), end = new Date(2026, 10, 9, 11)) {
  const view = mount(AvailabilityStrip, {
    props: { accountId: 'acc', attendees: ['lena@example.com', 'outside@elsewhere.test'], start, end },
  });
  await vi.advanceTimersByTimeAsync(300);
  await flushPromises();
  return view;
}

describe('AvailabilityStrip', () => {
  it('asks for the proposed day and draws a row per person', async () => {
    const view = await mountStrip();
    expect(backend.query).toHaveBeenCalledTimes(1);
    const input = backend.query.mock.calls[0]![0];
    expect(input).toMatchObject({ account_id: 'acc', attendees: ['lena@example.com', 'outside@elsewhere.test'], excluded_calendars: [] });
    expect(new Date(input.starts_at).getTime()).toBe(new Date(2026, 10, 9).getTime());
    expect(new Date(input.ends_at).getTime()).toBe(new Date(2026, 10, 10).getTime());
    expect(view.findAll('.strip__track')).toHaveLength(3);
    expect(view.findAll('.strip__track--unknown')).toHaveLength(1);
    // 10:00–11:00 touches the owner's 9–10 without overlap; Lena's 10:30–11:30 overlaps.
    expect(view.findAll('.strip__slot--conflict')).toHaveLength(1);
    expect(view.find('.strip__status').text()).toMatch(/^1 person is busy at /);
  });

  it('moves the slot a day either way and keeps its length', async () => {
    const view = await mountStrip();
    const [previous, next] = view.findAll('.strip__day button');
    await next!.trigger('click');
    await previous!.trigger('click');
    const moves = view.emitted('move')!;
    expect(moves[0]).toEqual([new Date(2026, 10, 10, 10), new Date(2026, 10, 10, 11)]);
    expect(moves[1]).toEqual([new Date(2026, 10, 8, 10), new Date(2026, 10, 8, 11)]);
  });

  it('moves the slot to the next time everyone known is free', async () => {
    backend.find.mockResolvedValue({
      time_zone: 'Europe/Berlin',
      unknown: ['outside@elsewhere.test'],
      days: [{ date: '2026-11-09', windows: [
        { starts_at: new Date(2026, 10, 9, 11, 30).toISOString(), ends_at: new Date(2026, 10, 9, 17).toISOString() },
      ] }],
    });
    const view = await mountStrip();
    await view.get('[data-testid="availability-next-free"]').trigger('click');
    await flushPromises();
    const input = backend.find.mock.calls[0]![0];
    expect(input).toMatchObject({ duration_minutes: 60, working_days: 10,
      working_hours: { start_minute: 540, end_minute: 1020, weekdays: [1, 2, 3, 4, 5] } });
    expect(new Date(input.from).getTime()).toBe(new Date(2026, 10, 9, 10, 1).getTime());
    expect(view.emitted('move')![0]).toEqual([new Date(2026, 10, 9, 11, 30), new Date(2026, 10, 9, 12, 30)]);
    expect(view.text()).toContain('Not counted, their availability is unknown: outside@elsewhere.test');
  });

  it('says so when nobody is free together instead of moving', async () => {
    backend.find.mockResolvedValue({ time_zone: 'UTC', unknown: [], days: [{ date: '2026-11-09', windows: [] }] });
    const view = await mountStrip();
    await view.get('[data-testid="availability-next-free"]').trigger('click');
    await flushPromises();
    expect(view.emitted('move')).toBeUndefined();
    expect(view.text()).toContain('No time when everyone is free in the next 10 working days.');
  });

  it('shows a load failure as a line, not a toast', async () => {
    backend.query.mockRejectedValueOnce(new Error('offline'));
    const view = await mountStrip();
    expect(view.find('.strip__status').text()).toBe('Availability could not be loaded.');
  });
});

describe('AvailabilityStrip overlaps', () => {
  it('draws and lists overlapping busy time once', async () => {
    backend.query.mockResolvedValue({
      own: [
        { starts_at: new Date(2026, 10, 9, 14).toISOString(), ends_at: new Date(2026, 10, 9, 16).toISOString(), tentative: false },
        { starts_at: new Date(2026, 10, 9, 14).toISOString(), ends_at: new Date(2026, 10, 9, 14, 30).toISOString(), tentative: true },
        { starts_at: new Date(2026, 10, 9, 15, 30).toISOString(), ends_at: new Date(2026, 10, 9, 17).toISOString(), tentative: false },
      ],
      attendees: [],
    });
    const view = await mountStrip();
    const own = view.findAll('.strip__track')[0]!;
    expect(own.findAll('.strip__busy')).toHaveLength(1);
    expect(own.find('.strip__busy--tentative').exists()).toBe(false);
    expect(own.text()).toMatch(/Busy 14:00–17:00$/);
  });
});

describe('AvailabilityStrip while a new report is on its way', () => {
  it('does not show the previous day as if it were the new one', async () => {
    const view = await mountStrip();
    expect(view.findAll('.strip__busy').length).toBeGreaterThan(0);
    let resolve!: (value: unknown) => void;
    backend.query.mockReturnValueOnce(new Promise((done) => { resolve = done; }));

    await view.setProps({ start: new Date(2026, 10, 10, 10), end: new Date(2026, 10, 10, 11) });
    // Before the debounce and while the request runs: no busy time, no verdict.
    expect(view.find('.strip__status').text()).toBe('Loading availability…');
    expect(view.findAll('.strip__busy')).toHaveLength(0);
    expect(view.find('.strip').classes()).toContain('strip--stale');
    await vi.advanceTimersByTimeAsync(300);
    expect(view.find('.strip__status').text()).toBe('Loading availability…');

    resolve({ own: [], attendees: [
      { address: 'lena@example.com', known: true, busy: [] },
      { address: 'outside@elsewhere.test', known: false, busy: [] },
    ] });
    await flushPromises();
    expect(view.find('.strip__status').text()).toBe('Everyone known is free at 10:00–11:00.');
    expect(view.find('.strip').classes()).not.toContain('strip--stale');
  });

  it('forgets a search answer when the day or the people change', async () => {
    backend.find.mockResolvedValue({ time_zone: 'UTC', unknown: ['outside@elsewhere.test'], days: [] });
    const view = await mountStrip();
    await view.get('[data-testid="availability-next-free"]').trigger('click');
    await flushPromises();
    expect(view.text()).toContain('No time when everyone is free');

    await view.setProps({ start: new Date(2026, 10, 10, 10), end: new Date(2026, 10, 10, 11) });
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(view.text()).not.toContain('No time when everyone is free');
    // Who could not be counted is still true on another day…
    expect(view.text()).toContain('Not counted, their availability is unknown');
    // …until the people change.
    await view.setProps({ attendees: ['lena@example.com'] });
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(view.text()).not.toContain('Not counted');
  });

  it('asks without the edited event and its saved slot', async () => {
    const editing = { id: 'event-1', start: new Date(2026, 10, 9, 10), end: new Date(2026, 10, 9, 11) };
    mount(AvailabilityStrip, {
      props: { accountId: 'acc', attendees: ['lena@example.com'], start: editing.start, end: editing.end, editing },
    });
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(backend.query.mock.calls[0]![0].editing).toEqual({
      event_id: 'event-1', starts_at: editing.start.toISOString(), ends_at: editing.end.toISOString(),
    });
  });
});

/**
 * A day on which this machine's own zone skips an hour, or a plain day in a
 * zone without DST. Vitest runs files in worker threads, where changing
 * `process.env.TZ` does not change the zone `Date` uses, so the test takes
 * the host's zone as it is (CI runs in UTC; locally it is wherever you are)
 * and asserts against what the strip itself draws.
 */
function springForwardDay(): Date {
  for (let day = new Date(2026, 0, 1); day.getFullYear() === 2026; day = new Date(day.getFullYear(), day.getMonth(), day.getDate() + 1)) {
    const next = new Date(day.getFullYear(), day.getMonth(), day.getDate() + 1);
    if (next.getTime() - day.getTime() < 24 * 3_600_000) return day;
  }
  return new Date(2026, 2, 29);
}

describe('AvailabilityStrip on a DST day', () => {
  it('moves the slot to the time under the pointer, not an hour away', async () => {
    const day = springForwardDay();
    const start = new Date(day.getFullYear(), day.getMonth(), day.getDate(), 10);
    const view = await mountStrip(start, new Date(start.getTime() + 60 * 60_000));
    const track = view.findAll('.strip__track')[0]!;
    const width = 200;
    track.element.getBoundingClientRect = () => ({ left: 0, width, top: 0, height: 20, right: width, bottom: 20, x: 0, y: 0, toJSON: () => ({}) });

    // A click right on an hour mark starts the slot at the time that mark is
    // labelled with. Past a skipped hour, mapping the click through the wall
    // clock instead of real time lands an hour off the label.
    const ticks = view.findAll('.strip__tick');
    expect(ticks.length).toBeGreaterThan(2);
    for (const tick of ticks.slice(1)) {
      const left = Number.parseFloat((tick.element as HTMLElement).style.left);
      track.element.dispatchEvent(new MouseEvent('pointerdown', { clientX: (left / 100) * width, bubbles: true }));
      const [moved, end] = view.emitted('move')!.at(-1) as [Date, Date];
      expect(formatTime(moved)).toBe(tick.text());
      expect(moved.getDate()).toBe(day.getDate());
      expect(end.getTime() - moved.getTime()).toBe(60 * 60_000);
    }
  });
});

describe('AvailabilityStrip at phone width', () => {
  it('gives no verdict while loading or after a failed load', async () => {
    let resolve!: (value: unknown) => void;
    backend.query.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
    useUiStore().isMobile = true;
    const view = mount(AvailabilityStrip, {
      props: { accountId: 'acc', attendees: ['lena@example.com'], start: new Date(2026, 10, 9, 10), end: new Date(2026, 10, 9, 11) },
    });
    await vi.advanceTimersByTimeAsync(300);
    expect(view.findAll('.strip__list-row')).toHaveLength(2);
    expect(view.findAll('.strip__verdict')).toHaveLength(0);
    expect(view.text()).not.toContain('Free then');
    expect(view.findAll('.strip__list-row--pending')).toHaveLength(2);

    resolve({ own: [], attendees: [{ address: 'lena@example.com', known: true, busy: [] }] });
    await flushPromises();
    expect(view.findAll('.strip__verdict').map((verdict) => verdict.text())).toEqual(['Free then', 'Free then']);

    // The next day fails: the status says so, the rows say nothing definite.
    backend.query.mockRejectedValueOnce(new Error('offline'));
    await view.setProps({ start: new Date(2026, 10, 10, 10), end: new Date(2026, 10, 10, 11) });
    await vi.advanceTimersByTimeAsync(300);
    await flushPromises();
    expect(view.find('.strip__status').text()).toBe('Availability could not be loaded.');
    expect(view.findAll('.strip__verdict')).toHaveLength(0);
    expect(view.findAll('.strip__times')).toHaveLength(0);
    expect(view.text()).not.toContain('Free then');
  });
});

