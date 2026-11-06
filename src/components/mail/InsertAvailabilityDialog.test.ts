// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';

const find = vi.hoisted(() => vi.fn());
vi.mock('@/services/backend/availability', () => ({ backendFindFreeSlots: find }));

import InsertAvailabilityDialog from './InsertAvailabilityDialog.vue';

enableAutoUnmount(afterEach);
beforeEach(() => {
  setActivePinia(createPinia());
  find.mockReset();
  find.mockResolvedValue({
    time_zone: 'UTC',
    unknown: [],
    days: [{ date: '2026-11-09', windows: [{ starts_at: '2026-11-09T09:00:00Z', ends_at: '2026-11-09T12:00:00Z' }] }],
  });
});

const mountDialog = () => mount(InsertAvailabilityDialog, { global: { stubs: { Teleport: true } } });

describe('InsertAvailabilityDialog', () => {
  it('asks for the owner alone with the chosen length and range, and inserts the text', async () => {
    const view = mountDialog();
    await flushPromises();
    expect(find).toHaveBeenCalledTimes(1);
    expect(find.mock.calls[0]![0]).toMatchObject({ account_id: null, attendees: [], duration_minutes: 30, working_days: 5,
      working_hours: { start_minute: 540, end_minute: 1020, weekdays: [1, 2, 3, 4, 5] } });
    const preview = view.get('[data-testid="insert-availability-preview"]').text();
    expect(preview).toContain("I'm free for 30 minutes at these times (");
    expect(preview).toContain('UTC');
    expect(preview).toMatch(/\n- Mon, Nov 9: 09:00\s*[–-]\s*12:00$/);

    await view.get('[data-testid="insert-availability-submit"]').trigger('click');
    const [content] = view.emitted('insert')![0] as [{ text: string; html: string }];
    expect(content.text).toBe(preview);
    expect(content.html).toMatch(/^<p>I'm free for 30 minutes at these times \(.*\):<\/p><ul><li><p>Mon, Nov 9: /);
  });

  it('asks again when the length or the range changes', async () => {
    const view = mountDialog();
    await flushPromises();
    const groups = view.findAll('.segmented');
    await groups[0]!.findAll('button').find((button) => button.text() === '1 h')!.trigger('click');
    await flushPromises();
    await groups[1]!.findAll('button').find((button) => button.text() === '10 working days')!.trigger('click');
    await flushPromises();
    expect(find.mock.calls.map((call) => [call[0].duration_minutes, call[0].working_days])).toEqual([[30, 5], [60, 5], [60, 10]]);
    expect(view.get('[data-testid="insert-availability-preview"]').text()).toContain('for 1 hour');
  });

  it('offers nothing to insert when there is no free time', async () => {
    find.mockResolvedValue({ time_zone: 'UTC', unknown: [], days: [{ date: '2026-11-09', windows: [] }] });
    const view = mountDialog();
    await flushPromises();
    expect(view.text()).toContain('No free time within working hours in the next 5 working days.');
    expect(view.get('[data-testid="insert-availability-submit"]').attributes('disabled')).toBeDefined();
  });
});
