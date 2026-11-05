import { describe, expect, it } from 'vitest';
import type { CalendarItem } from './calendarGrid';
import { todayAgenda } from './todayAgenda';

function item(key: string, start: Date, end: Date, overrides: Partial<CalendarItem> = {}): CalendarItem {
  return {
    key,
    kind: 'event',
    id: key,
    uid: key,
    title: key,
    location: null,
    description: null,
    organizer: null,
    attendees: [],
    start,
    end,
    allDay: false,
    status: 'own',
    source: 'local',
    sourceKey: 'local',
    color: null,
    ...overrides,
  };
}

const at = (hour: number, minute = 0) => new Date(2026, 10, 5, hour, minute);
const allDay = (key: string) =>
  item(key, new Date(2026, 10, 5), new Date(2026, 10, 6), { allDay: true });

describe('todayAgenda', () => {
  // The owner's Thursday: three all-day entries used to fill every slot.
  const day = [
    allDay('Ole in Kur'),
    allDay('Kochtag'),
    allDay('Kinderbetreuung'),
    item('Sprint review', at(11), at(12)),
    item('Podologie', at(13, 45), at(14, 30)),
    item('Kinderbetreuung Abholung', at(14), at(16)),
    item('Client call', at(16, 30), at(17, 15)),
    item('Retro', at(17, 30), at(18, 15)),
  ];

  it('puts running and upcoming timed events first and drops the ones that are over', () => {
    const agenda = todayAgenda(day, at(14, 10), 3);
    expect(agenda.timed.map((row) => [row.item.key, row.current])).toEqual([
      ['Podologie', true],
      ['Kinderbetreuung Abholung', true],
      ['Client call', false],
    ]);
    expect(agenda.moreTimed).toBe(1);
  });

  it('keeps all-day entries apart, so they cannot crowd out the timed ones', () => {
    const agenda = todayAgenda(day, at(8), 3);
    expect(agenda.allDay.map((entry) => entry.key)).toEqual(['Ole in Kur', 'Kochtag', 'Kinderbetreuung']);
    expect(agenda.timed).toHaveLength(3);
    expect(agenda.timed.every((row) => !row.current)).toBe(true);
    expect(agenda.moreTimed).toBe(2);
  });

  it('treats an event that ends right now as over', () => {
    const agenda = todayAgenda(day, at(18, 15), 3);
    expect(agenda.timed).toEqual([]);
    expect(agenda.allDay).toHaveLength(3);
  });

  it('counts a timed event spanning midnight with the all-day entries', () => {
    const overnight = item('Night shift', new Date(2026, 10, 4, 22), at(6));
    const agenda = todayAgenda([overnight], at(2), 3);
    expect(agenda.allDay.map((entry) => entry.key)).toEqual(['Night shift']);
    expect(agenda.timed).toEqual([]);
  });

  it('leaves out declined and cancelled meetings and other days', () => {
    const agenda = todayAgenda(
      [
        item('Declined', at(15), at(16), { status: 'declined' }),
        item('Cancelled', at(15), at(16), { status: 'cancelled' }),
        item('Tomorrow', new Date(2026, 10, 6, 9), new Date(2026, 10, 6, 10)),
        item('Pending', at(15), at(16), { status: 'needs-action' }),
      ],
      at(9),
      3,
    );
    expect(agenda.timed.map((row) => row.item.key)).toEqual(['Pending']);
    expect(agenda.allDay).toEqual([]);
  });
});
