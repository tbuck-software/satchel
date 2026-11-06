// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';

const invites = vi.hoisted(() => ({
  items: [],
  events: [] as Array<Record<string, unknown>>,
  sources: [] as Array<Record<string, unknown>>,
  load: vi.fn(),
  syncRemote: vi.fn(),
}));

vi.mock('@/stores/invites', () => ({ useInvitesStore: () => invites }));
vi.mock('@/stores/calendarUi', () => ({
  useCalendarUiStore: () => ({
    isSourceHidden: vi.fn(() => false),
    toggleSource: vi.fn(),
    workingHours: { startMinute: 540, endMinute: 1020, weekdays: [1, 2, 3, 4, 5] },
    countsForAvailability: vi.fn(() => true),
    toggleAvailability: vi.fn(),
    setWorkingHours: vi.fn(),
  }),
}));

import CalendarSettings from './CalendarSettings.vue';

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  invites.syncRemote.mockResolvedValue([]);
  invites.events = [];
  invites.sources = [];
});

describe('calendar sync errors', () => {
  it('translates an encoded backend error instead of displaying its JSON', async () => {
    const encoded = JSON.stringify({
      key: 'backend.errors.googleCalendarSyncFailed',
      params: { account: 'broken@example.test' },
    });
    invites.syncRemote.mockResolvedValue([encoded]);
    const wrapper = mount(CalendarSettings);

    const sync = wrapper.findAll('button').find((button) => button.text().includes('Sync now'));
    expect(sync).toBeDefined();
    await sync!.trigger('click');

    await vi.waitFor(() =>
      expect(wrapper.get('[role="status"]').text()).toContain(
        'Google Calendar for broken@example.test could not be synchronized.',
      ),
    );
    expect(wrapper.get('[role="status"]').text()).not.toContain('{"key"');
  });
});

describe('calendar list', () => {
  it('names Outlook calendars as Outlook declares them, events or not', () => {
    invites.sources = [
      { account_id: 'outlook', source: 'outlook', remote_id: 'AAMkAGI2-main', name: 'Calendar' },
      { account_id: 'outlook', source: 'outlook', remote_id: 'AAMkAGI2-team', name: 'Team' },
    ];
    invites.events = [{
      id: 'outlook-event-1', account_id: 'outlook', uid: 'uid-1', title: 'Standup',
      starts_at: '2026-07-14T08:00:00Z', ends_at: '2026-07-14T08:15:00Z', all_day: 0,
      attendees_json: '[]', source: 'outlook', collection_href: 'AAMkAGI2-main',
      created_at: '2026-07-01T00:00:00Z', updated_at: '2026-07-01T00:00:00Z',
    }];
    const wrapper = mount(CalendarSettings);

    const rows = wrapper.findAll('.source-row').map((row) => row.text());
    expect(rows).toHaveLength(2);
    expect(rows[0]).toContain('Calendar');
    expect(rows[0]).toContain('one event');
    expect(rows[1]).toContain('Team');
  });
});
