// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { createI18n } from 'vue-i18n';
import type { CalendarItem } from '@/utils/calendarGrid';
import CalendarWeekGrid from './CalendarWeekGrid.vue';

function i18n(locale: 'en' | 'de') {
  return createI18n({ legacy: false, locale, messages: { en: {}, de: {} } });
}

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
      global: { plugins: [i18n('en')] },
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

describe('CalendarWeekGrid headers', () => {
  // 2026-10-26 is a Monday.
  const week = Array.from({ length: 7 }, (_, index) => new Date(2026, 9, 26 + index));

  it.each([
    ['en', 'Mon', 'Sun'],
    ['de', 'Mo', 'So'],
  ] as const)('names the weekdays in the UI locale (%s)', (locale, first, last) => {
    const wrapper = mount(CalendarWeekGrid, {
      props: { days: week, items: [] },
      global: { plugins: [i18n(locale)] },
    });

    const names = wrapper.findAll('.week__dayname').map((node) => node.text());
    expect(names).toHaveLength(7);
    expect(names[0]).toContain(first);
    expect(names[6]).toContain(last);

    wrapper.unmount();
  });
});

describe('CalendarWeekGrid short events', () => {
  it('keeps the title of a half-hour event on one line with its time', () => {
    const coffee = makeItem('coffee', 'code.example coffee', 14, 14);
    coffee.end = new Date(2026, 7, 12, 14, 30);
    const wrapper = mount(CalendarWeekGrid, {
      props: { days: [new Date(2026, 7, 12)], items: [coffee, makeItem('long', 'Workshop', 9, 11)] },
      global: { plugins: [i18n('en')] },
    });

    const [long, short] = wrapper.findAll('.week__event');
    expect(short!.classes()).toContain('week__event--compact');
    expect(short!.get('.week__event-title').text()).toBe('code.example coffee');
    expect(long!.classes()).not.toContain('week__event--compact');

    wrapper.unmount();
  });
});
