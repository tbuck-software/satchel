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
