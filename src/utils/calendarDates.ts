/** Value for an `<input type="date">`, in local time. */
export function toDateInput(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** DATE values are calendar dates, including legacy UTC-midnight mirrors. */
export function parseCalendarBoundary(value: string, allDay: boolean): Date {
  if (!allDay) return new Date(value);
  const date = value.slice(0, 10);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return new Date(NaN);
  const parsed = new Date(`${date}T00:00`);
  return toDateInput(parsed) === date ? parsed : new Date(NaN);
}
