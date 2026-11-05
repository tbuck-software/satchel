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
  it('cascades overlapping events from their column\'s left edge', () => {
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
    // jsdom reorders the arguments of `min()`/`max()`; match loosely.
    expect(events).toHaveLength(2);
    expect(events[0]!.text()).toContain('Langer Termin mit lesbarem Titel');
    expect(events[0]!.element.style.left).toBe('0%');
    expect(events[1]!.element.style.left).toBe('50%');
    // Two columns: the first reaches 1.7 columns (85%), the second ends at
    // the edge; neither gets narrower than 56px nor wider than its room.
    expect(events[0]!.element.style.width).toMatch(/85%.*56px|56px.*85%/);
    expect(events[0]!.element.style.width).toContain('100%');
    expect(events[1]!.element.style.width).toMatch(/50%.*56px|56px.*50%/);
    expect(events[0]!.element.style.zIndex).toBe('1');
    expect(events[1]!.element.style.zIndex).toBe('2');
    expect(events.every((event) => event.classes().includes('week__event--shared'))).toBe(true);

    wrapper.unmount();
  });

  it('keeps a lone event on the full width', () => {
    const wrapper = mount(CalendarWeekGrid, {
      props: { days: [new Date(2026, 7, 12)], items: [makeItem('solo', 'Allein', 8, 9)] },
      global: { plugins: [i18n('en')] },
    });

    const event = wrapper.get<HTMLElement>('.week__event');
    expect(event.element.style.left).toBe('0px');
    expect(event.element.style.right).toBe('1px');
    expect(event.element.style.width).toBe('');
    expect(event.classes()).not.toContain('week__event--shared');

    wrapper.unmount();
  });

  it('gives the title as many lines as the block has room for', () => {
    const wrapper = mount(CalendarWeekGrid, {
      props: {
        days: [new Date(2026, 7, 12)],
        items: [makeItem('two-hours', 'Kinderbetreuung Herbstferien', 14, 16), makeItem('hour', 'Kurz', 9, 10)],
      },
      global: { plugins: [i18n('en')] },
    });

    const [hour, twoHours] = wrapper.findAll<HTMLElement>('.week__event');
    // 96px block, 6px padding, 15px lines → 6; 48px → 2.
    expect(twoHours!.element.style.getPropertyValue('--title-lines')).toBe('6');
    expect(hour!.element.style.getPropertyValue('--title-lines')).toBe('2');

    wrapper.unmount();
  });
});

describe('CalendarWeekGrid now line', () => {
  it('runs under the events with a single dot on top', () => {
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const wrapper = mount(CalendarWeekGrid, {
      props: { days: [today], items: [] },
      global: { plugins: [i18n('en')] },
    });

    expect(wrapper.findAll('.week__now')).toHaveLength(1);
    expect(wrapper.findAll('.week__now-dot')).toHaveLength(1);

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
