import { isMultiDay, startOfDay, addDays, type CalendarItem } from './calendarGrid';

/**
 * What the sidebar's Today card shows: what is on now and what comes next,
 * like Google's "what's next" widget. All-day entries are the backdrop of the
 * day, not an appointment, so they are folded into one line instead of
 * taking the slots the timed events need. Timed events that are over drop
 * out; a running one is marked as current.
 */
export interface TodayAgenda {
  /** All-day and multi-day entries touching today, in start order. */
  allDay: CalendarItem[];
  /** Timed entries still running or yet to start, at most `maxTimed`. */
  timed: Array<{ item: CalendarItem; current: boolean }>;
  /** Timed entries still ahead that did not fit. */
  moreTimed: number;
}

export function todayAgenda(items: CalendarItem[], now: Date, maxTimed: number): TodayAgenda {
  const dayStart = startOfDay(now).getTime();
  const dayEnd = addDays(startOfDay(now), 1).getTime();
  const nowMs = now.getTime();
  const today = items
    // A declined or cancelled meeting is not part of the day.
    .filter((item) => item.status !== 'declined' && item.status !== 'cancelled')
    .filter((item) => item.start.getTime() < dayEnd && item.end.getTime() > dayStart)
    .sort((a, b) => a.start.getTime() - b.start.getTime() || a.end.getTime() - b.end.getTime());

  const allDay = today.filter((item) => item.allDay || isMultiDay(item));
  const ahead = today
    .filter((item) => !item.allDay && !isMultiDay(item))
    .filter((item) => item.end.getTime() > nowMs)
    .map((item) => ({ item, current: item.start.getTime() <= nowMs }));

  return {
    allDay,
    timed: ahead.slice(0, maxTimed),
    moreTimed: Math.max(0, ahead.length - maxTimed),
  };
}
