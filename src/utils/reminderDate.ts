import { currentLocale, t } from '@/i18n/runtime';

export type ReminderLanguage = 'de' | 'en';

export interface ReminderSuggestion {
  id: string;
  label: string;
  detail: string;
  date: Date;
  source: 'parsed' | 'preset';
}

const WEEKDAYS: Array<{ day: number; aliases: string[] }> = [
  { day: 0, aliases: ['sonntag', 'sunday', 'sun', 'so'] },
  { day: 1, aliases: ['montag', 'monday', 'mon', 'mo'] },
  { day: 2, aliases: ['dienstag', 'tuesday', 'tue', 'di'] },
  { day: 3, aliases: ['mittwoch', 'wednesday', 'wed', 'mi'] },
  { day: 4, aliases: ['donnerstag', 'thursday', 'thu', 'do'] },
  { day: 5, aliases: ['freitag', 'friday', 'fri', 'fr'] },
  { day: 6, aliases: ['samstag', 'saturday', 'sat', 'sa'] },
];

function normalized(value: string): string {
  return value.trim().toLocaleLowerCase().replace(/\s+/g, ' ');
}

function cloneAt(base: Date, hour: number, minute = 0): Date {
  const date = new Date(base);
  date.setHours(hour, minute, 0, 0);
  return date;
}

function addDays(base: Date, days: number): Date {
  const date = new Date(base);
  date.setDate(date.getDate() + days);
  return date;
}

function nextWeekday(base: Date, weekday: number, hour: number, minute: number): Date {
  const days = ((weekday - base.getDay() + 7) % 7) || 7;
  return cloneAt(addDays(base, days), hour, minute);
}

function inferredLanguage(input: string, fallback: ReminderLanguage): ReminderLanguage {
  const value = normalized(input);
  if (/\b(today|tomorrow|tonight|next|in|hour|hours|day|days|week|weeks|later|morning|afternoon|evening)\b/.test(value)) return 'en';
  if (/\b(heute|morgen|übermorgen|uebermorgen|nächste|naechste|in|stunde|stunden|tag|tage|woche|wochen|später|spaeter|vormittag|nachmittag|abend)\b/.test(value)) return 'de';
  return fallback;
}

function timeFromInput(input: string, fallbackHour: number): { hour: number; minute: number } {
  const value = normalized(input);
  const match = value.match(/(?:\b(?:at|um)\s*)?\b(\d{1,2})(?:[:.](\d{2}))?\s*(am|pm|uhr)?\b/);
  if (!match) {
    if (/\b(tonight|evening|abend)\b/.test(value)) return { hour: 18, minute: 0 };
    if (/\b(afternoon|nachmittag)\b/.test(value)) return { hour: 14, minute: 0 };
    if (/\b(morning|vormittag|morgens)\b/.test(value)) return { hour: 8, minute: 0 };
    return { hour: fallbackHour, minute: 0 };
  }

  let hour = Number(match[1]);
  const minute = Number(match[2] ?? 0);
  if (match[3] === 'pm' && hour < 12) hour += 12;
  if (match[3] === 'am' && hour === 12) hour = 0;
  if (hour > 23 || minute > 59) return { hour: fallbackHour, minute: 0 };
  return { hour, minute };
}

function parseRelative(input: string, now: Date): Date | null {
  const value = normalized(input);
  const match = value.match(/\bin\s+(\d+)\s*(min(?:ute)?s?|minuten?|h(?:ours?)?|stunden?|d(?:ays?)?|tage?n?|w(?:eeks?)?|wochen?)\b/);
  if (!match?.[1] || !match[2]) return null;
  const amount = Number(match[1]);
  const unit = match[2];
  const milliseconds = unit.startsWith('min')
    ? amount * 60_000
    : /^(h|hour|stunde)/.test(unit)
      ? amount * 60 * 60_000
      : /^(w|week|woche)/.test(unit)
        ? amount * 7 * 24 * 60 * 60_000
        : amount * 24 * 60 * 60_000;
  return new Date(now.getTime() + milliseconds);
}

function parseCalendarDate(input: string, now: Date): Date | null {
  const value = normalized(input);
  const iso = value.match(/\b(20\d{2})-(\d{1,2})-(\d{1,2})\b/);
  const german = value.match(/\b(\d{1,2})\.(\d{1,2})(?:\.(\d{2,4}))?\b/);
  const match = iso ?? german;
  if (!match) return null;

  let year: number;
  let month: number;
  let day: number;
  if (iso) {
    year = Number(match[1]);
    month = Number(match[2]) - 1;
    day = Number(match[3]);
  } else {
    day = Number(match[1]);
    month = Number(match[2]) - 1;
    year = match[3] ? Number(match[3]) : now.getFullYear();
    if (year < 100) year += 2000;
  }
  const { hour, minute } = timeFromInput(value.replace(match[0], ''), 8);
  let date = new Date(year, month, day, hour, minute, 0, 0);
  if (!match[3] && date.getTime() <= now.getTime()) {
    date = new Date(year + 1, month, day, hour, minute, 0, 0);
  }
  if (date.getMonth() !== month || date.getDate() !== day) return null;
  return date;
}

export function parseReminderDate(input: string, now = new Date()): Date | null {
  const value = normalized(input);
  if (!value) return null;

  const relative = parseRelative(value, now);
  if (relative) return relative;

  const calendar = parseCalendarDate(value, now);
  if (calendar) return calendar.getTime() > now.getTime() ? calendar : null;

  if (/\b(later|später|spaeter)\b/.test(value)) {
    return new Date(now.getTime() + 2 * 60 * 60_000);
  }

  const { hour, minute } = timeFromInput(value, 8);
  let date: Date | null = null;
  if (/(?:^|\s)(day after tomorrow|übermorgen|uebermorgen)(?:\s|$)/.test(value)) {
    date = cloneAt(addDays(now, 2), hour, minute);
  } else if (/\b(tomorrow|morgen)\b/.test(value)) {
    date = cloneAt(addDays(now, 1), hour, minute);
  } else if (/\b(today|heute|tonight)\b/.test(value)) {
    date = cloneAt(now, hour, minute);
  } else if (/\b(next week|nächste woche|naechste woche)\b/.test(value)) {
    date = nextWeekday(now, 1, hour, minute);
  } else if (/\b(next month|nächsten monat|naechsten monat)\b/.test(value)) {
    date = new Date(now.getFullYear(), now.getMonth() + 1, 1, hour, minute, 0, 0);
  } else {
    const weekday = WEEKDAYS.find((entry) =>
      entry.aliases.some((alias) => new RegExp(`\\b${alias}\\b`).test(value)),
    );
    if (weekday) date = nextWeekday(now, weekday.day, hour, minute);
  }

  if (!date && /\b\d{1,2}(?::\d{2})?\s*(?:am|pm|uhr)?\b/.test(value)) {
    date = cloneAt(now, hour, minute);
    if (date.getTime() <= now.getTime()) date = addDays(date, 1);
  }
  return date && date.getTime() > now.getTime() ? date : null;
}

export function formatReminderDate(date: Date, language: ReminderLanguage): string {
  return new Intl.DateTimeFormat(language === 'de' ? 'de-DE' : 'en-US', {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    hour: '2-digit',
    minute: '2-digit',
  }).format(date);
}

function suggestion(
  id: string,
  label: string,
  date: Date,
  language: ReminderLanguage,
  source: ReminderSuggestion['source'],
): ReminderSuggestion {
  return { id, label, date, source, detail: formatReminderDate(date, language) };
}

export function reminderSuggestions(
  input: string,
  now = new Date(),
  fallbackLanguage: ReminderLanguage = currentLocale(),
): ReminderSuggestion[] {
  const language = inferredLanguage(input, fallbackLanguage);
  const parsed = parseReminderDate(input, now);
  if (parsed) {
    return [suggestion(
      `parsed:${parsed.toISOString()}`,
      t('calendar.reminder.set'),
      parsed,
      language,
      'parsed',
    )];
  }
  if (input.trim()) return [];

  const today = cloneAt(now, 14);
  const presets = [
    ...(today.getTime() > now.getTime()
      ? [suggestion('today', t('calendar.reminder.thisAfternoon'), today, language, 'preset')]
      : []),
    suggestion('tomorrow', t('calendar.reminder.tomorrowMorning'), cloneAt(addDays(now, 1), 8), language, 'preset'),
    suggestion('next-week', t('calendar.reminder.nextWeek'), nextWeekday(now, 1, 8, 0), language, 'preset'),
    suggestion('next-month', t('calendar.reminder.nextMonth'), new Date(now.getFullYear(), now.getMonth() + 1, 1, 8), language, 'preset'),
  ];
  return presets;
}
