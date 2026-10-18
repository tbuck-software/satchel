// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';

const invites = vi.hoisted(() => ({
  items: [],
  events: [],
  load: vi.fn(),
  syncRemote: vi.fn(),
}));

vi.mock('@/stores/invites', () => ({ useInvitesStore: () => invites }));
vi.mock('@/stores/calendarUi', () => ({
  useCalendarUiStore: () => ({
    isSourceHidden: vi.fn(() => false),
    toggleSource: vi.fn(),
  }),
}));

import CalendarSettings from './CalendarSettings.vue';

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  invites.syncRemote.mockResolvedValue([]);
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
