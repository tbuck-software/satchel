/** Local wall-clock values for the scheduling menu and custom picker. */
export function sendLaterPresets(now = new Date()) {
  const tomorrow = (hour: number) => {
    const date = new Date(now);
    date.setDate(date.getDate() + 1);
    date.setHours(hour, 0, 0, 0);
    return date;
  };
  const monday = new Date(now);
  monday.setDate(monday.getDate() + ((1 + 7 - monday.getDay()) % 7 || 7));
  monday.setHours(9, 0, 0, 0);
  return [
    { day: 'tomorrow', date: tomorrow(9) },
    { day: 'tomorrow', date: tomorrow(15) },
    { day: 'monday', date: monday },
  ] as const;
}

export function toLocalMinute(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
