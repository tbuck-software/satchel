/** Wall-clock working hours: minutes after midnight and ISO weekdays (1 = Monday). */
export interface WorkingHours {
  startMinute: number;
  endMinute: number;
  weekdays: number[];
}

/** Monday to Friday, nine to five, until someone says otherwise. */
export function defaultWorkingHours(): WorkingHours {
  return { startMinute: 9 * 60, endMinute: 17 * 60, weekdays: [1, 2, 3, 4, 5] };
}

/** A stored or typed value as working hours, or `null` when it cannot be one. */
export function normalizeWorkingHours(value: unknown): WorkingHours | null {
  if (typeof value !== 'object' || value === null) return null;
  const { startMinute, endMinute, weekdays } = value as Record<string, unknown>;
  if (!Number.isInteger(startMinute) || !Number.isInteger(endMinute) || !Array.isArray(weekdays)) return null;
  const start = startMinute as number;
  const end = endMinute as number;
  if (start < 0 || end > 24 * 60 || start >= end) return null;
  const days = [...new Set(weekdays.filter((day): day is number => Number.isInteger(day) && day >= 1 && day <= 7))]
    .sort((a, b) => a - b);
  if (days.length === 0) return null;
  return { startMinute: start, endMinute: end, weekdays: days };
}

/** `HH:MM` for a time input. 1440 reads as 24:00, which inputs cannot show, so 23:59. */
export function minuteToTimeInput(minute: number): string {
  const clamped = Math.min(Math.max(minute, 0), 24 * 60 - 1);
  return `${String(Math.floor(clamped / 60)).padStart(2, '0')}:${String(clamped % 60).padStart(2, '0')}`;
}

/** Minutes after midnight from a time input, or `null`. `23:59` as an end means midnight. */
export function timeInputToMinute(value: string, isEnd = false): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  const total = hours * 60 + minutes;
  return isEnd && total === 24 * 60 - 1 ? 24 * 60 : total;
}

/** The IANA zone this device runs in; free slots are computed and stated in it. */
export function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

/** The wire form `find_free_slots` takes. */
export function workingHoursInput(hours: WorkingHours) {
  return { start_minute: hours.startMinute, end_minute: hours.endMinute, weekdays: [...hours.weekdays] };
}
