// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { createI18n } from 'vue-i18n';
import type { CalendarItem } from '@/utils/calendarGrid';
import CalendarDayAgenda from './CalendarDayAgenda.vue';
import CalendarMonthGrid from './CalendarMonthGrid.vue';

const i18n = () => createI18n({ legacy: false, locale: 'en', messages: { en: {} } });

function makeItem(key: string, title: string, day: number, hour: number): CalendarItem {
  return {
    key, kind: 'event', id: key, uid: key, title,
    location: null, description: null, organizer: null, attendees: [],
    start: new Date(2026, 9, day, hour), end: new Date(2026, 9, day, hour + 1),
    allDay: false, status: 'own', source: 'local', sourceKey: 'local', color: null,
  };
}

describe('the phone month', () => {
  const items = [makeItem('a', 'Sprint review', 27, 11), makeItem('b', 'Retro', 27, 17), makeItem('c', 'Lunch', 28, 12)];

  it('draws days as numbers and dots, and picks a day instead of opening it', async () => {
    const wrapper = mount(CalendarMonthGrid, {
      props: { anchor: new Date(2026, 9, 27), items, compact: true },
      global: { plugins: [i18n()] },
    });

    expect(wrapper.find('.event-chip').exists()).toBe(false);
    const picked = wrapper.get('.day--picked');
    expect(picked.text()).toBe('27');
    expect(picked.findAll('.day__dot')).toHaveLength(2);

    const next = wrapper.findAll('.day').find((cell) => cell.text() === '28' && !cell.classes('day--muted'))!;
    await next.trigger('click');
    expect(wrapper.emitted('pickDay')?.[0]?.[0]).toEqual(new Date(2026, 9, 28));
    expect(wrapper.emitted('openDay')).toBeUndefined();
  });

  it('lists the picked day with times, and says when it is empty', () => {
    const wrapper = mount(CalendarDayAgenda, {
      props: { days: [new Date(2026, 9, 27), new Date(2026, 9, 26)], items },
      global: { plugins: [i18n()] },
    });

    const [busy, free] = wrapper.findAll('.agenda__day');
    expect(busy!.findAll('.agenda__title').map((title) => title.text())).toEqual(['Sprint review', 'Retro']);
    expect(free!.find('.agenda__row').exists()).toBe(false);
    expect(free!.find('.agenda__empty').exists()).toBe(true);
  });
});
