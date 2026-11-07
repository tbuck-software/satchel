import type { BackendFreeSlotsReport } from '@/services/backend/availability';
import { datetimeFormats } from '@/i18n/formats';

type Translate = (key: string, named?: Record<string, unknown>, plural?: number) => string;

export interface AvailabilityLines {
  intro: string;
  /** One line per day with free time: "Mon, Nov 9: 9:00 – 11:30 AM, 2:00 – 5:00 PM". */
  days: string[];
}

/** "30 minutes", "1 hour", "90 minutes": whole hours read as hours. */
export function durationText(t: Translate, minutes: number): string {
  return minutes % 60 === 0
    ? t('availability.text.hours', { count: minutes / 60 }, minutes / 60)
    : t('availability.text.minutes', { count: minutes }, minutes);
}

/** "Central European Time, Europe/Berlin", or the zone id alone. */
export function zoneLabel(zone: string, locale: string, at: Date): string {
  for (const style of ['longGeneric', 'long'] as const) {
    try {
      const name = new Intl.DateTimeFormat(locale, { timeZone: zone, timeZoneName: style })
        .formatToParts(at)
        .find((part) => part.type === 'timeZoneName')?.value;
      if (name && name !== zone) return plainSpaces(`${name}, ${zone}`);
    } catch {
      // An engine without the style tries the next one.
    }
  }
  return zone;
}

/** The app's own time format (the one the calendar shows), read in `zone`. */
function timeFormat(locale: string, zone: string): Intl.DateTimeFormat {
  const app = datetimeFormats[locale]?.time ?? datetimeFormats.en?.time ?? { hour: '2-digit', minute: '2-digit' };
  return new Intl.DateTimeFormat(locale, { ...(app as Intl.DateTimeFormatOptions), timeZone: zone });
}

/**
 * ICU puts thin and narrow no-break spaces around range dashes and before
 * AM/PM. Plain-text mail shows them as gaps of odd width, or as boxes in
 * older clients and fonts; plain spaces read the same everywhere. The en
 * dash stays: it is ordinary UTF-8 text in every client that shows mail.
 */
function plainSpaces(text: string): string {
  return text.replace(/[\u00a0\u2009\u202f]/g, ' ');
}

function timeRange(locale: string, zone: string, start: Date, end: Date): string {
  const format = timeFormat(locale, zone);
  return plainSpaces(typeof format.formatRange === 'function'
    ? format.formatRange(start, end)
    : `${format.format(start)} – ${format.format(end)}`);
}

/**
 * The free windows of a report as short lines of text in the report's time
 * zone, or `null` when no day has any.
 */
export function availabilityLines(
  report: BackendFreeSlotsReport,
  options: { t: Translate; locale: string; durationMinutes: number },
): AvailabilityLines | null {
  const { t, locale, durationMinutes } = options;
  const zone = report.time_zone;
  const dayFormat = new Intl.DateTimeFormat(locale, { weekday: 'short', month: 'short', day: 'numeric', timeZone: zone });
  const days = report.days
    .filter((day) => day.windows.length > 0)
    .map((day) => {
      const first = new Date(day.windows[0]!.starts_at);
      const times = day.windows
        .map((window) => timeRange(locale, zone, new Date(window.starts_at), new Date(window.ends_at)))
        .join(', ');
      return t('availability.text.day', { day: plainSpaces(dayFormat.format(first)), times });
    });
  if (days.length === 0) return null;
  const firstStart = new Date(report.days.find((day) => day.windows.length > 0)!.windows[0]!.starts_at);
  return {
    intro: t('availability.text.intro', {
      duration: durationText(t, durationMinutes),
      zone: zoneLabel(zone, locale, firstStart),
    }),
    days,
  };
}

/** For a plain-text body: the intro, then one "- " line per day. */
export function availabilityPlainText(lines: AvailabilityLines): string {
  return [lines.intro, ...lines.days.map((day) => `- ${day}`)].join('\n');
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/** For the rich editor: a paragraph and a plain bullet list. */
export function availabilityHtml(lines: AvailabilityLines): string {
  const items = lines.days.map((day) => `<li><p>${escapeHtml(day)}</p></li>`).join('');
  return `<p>${escapeHtml(lines.intro)}</p><ul>${items}</ul>`;
}
