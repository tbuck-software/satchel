// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import type { CalendarItem } from '@/utils/calendarGrid';
import CalendarWeekGrid from './CalendarWeekGrid.vue';

function makeItem(key: string, title: string, startHour: number, endHour: number): CalendarItem {
  return {
    key,
    kind: 'event',
    id: key,
    uid: key,
    title,
    location: null,
    description: null,
    organizer: null,
    attendees: [],
    start: new Date(2026, 7, 12, startHour),
    end: new Date(2026, 7, 12, endHour),
    allDay: false,
    status: 'own',
    source: 'local',
    sourceKey: 'local',
    color: 'var(--accent-purple)',
  };
}

describe('CalendarWeekGrid overlapping events', () => {
  it('keeps stacked events wide and insets the later one by 10px', () => {
    const wrapper = mount(CalendarWeekGrid, {
      props: {
        days: [new Date(2026, 7, 12)],
        items: [
          makeItem('earlier', 'Langer Termin mit lesbarem Titel', 8, 11),
          makeItem('later', 'Späterer Termin', 9, 10),
        ],
      },
    });

    const events = wrapper.findAll<HTMLElement>('.week__event');
    expect(events).toHaveLength(2);
    expect(events[0]!.text()).toContain('Langer Termin mit lesbarem Titel');
    expect(events[0]!.element.style.left).toBe('0px');
    expect(events[0]!.element.style.right).toBe('1px');
    expect(events[0]!.element.style.width).toBe('');
    expect(events[1]!.element.style.left).toBe('10px');
    expect(events[1]!.classes()).toContain('week__event--overlapping');

    wrapper.unmount();
  });
});
