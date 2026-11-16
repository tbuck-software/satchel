// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import EventDetailsPopover from './EventDetailsPopover.vue';
import { itemFromEvent } from '@/utils/calendarGrid';

const outlook = (read_only: number) => itemFromEvent({
  id: 'outlook-event-1', uid: 'uid', account_id: 'outlook', title: 'Board meeting',
  starts_at: '2026-07-14T14:00:00Z', ends_at: '2026-07-14T15:00:00Z', all_day: 0,
  attendees_json: '[]', source: 'outlook', collection_href: 'cal-main', description: null,
  location: null, created_at: '', updated_at: '', read_only, recurrence_instance: 0, rich_description: 0,
});

describe('event details', () => {
  it('offers edit and delete for an Outlook event the account may change', () => {
    const view = mount(EventDetailsPopover, { props: { item: outlook(0) } });
    expect(view.find('[title="Edit event"]').exists()).toBe(true);
    expect(view.find('[title="Delete event"]').exists()).toBe(true);
    expect(view.text()).toContain('Outlook Calendar');
  });

  it('leaves a meeting the account only attends to Outlook', () => {
    const view = mount(EventDetailsPopover, { props: { item: outlook(1) } });
    expect(view.find('[title="Edit event"]').exists()).toBe(false);
    expect(view.find('[title="Delete event"]').exists()).toBe(false);
    expect(view.text()).toContain('Change this event in Outlook');
  });
});
