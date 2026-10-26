import type { BackendCalendarEvent, BackendCalendarInvite } from '@/services/backend';
import { currentLocale, d, t } from '@/i18n/runtime';

/**
 * Shared model and date math for the calendar views. Invites and locally
 * created events render through the same grid, so both are normalised into
 * `CalendarItem` first.
 */

export type CalendarItemStatus =
  | 'own'
  | 'accepted'
  | 'tentative'
  | 'needs-action'
  | 'declined'
  | 'cancelled';

export interface CalendarItem {
  /** Stable render key across both sources (`event:<id>` / `invite:<id>`). */
  key: string;
  kind: 'event' | 'invite';
  id: string;
  uid: string;
  title: string;
  location: string | null;
  description: string | null;
  organizer: string | null;
  attendees: string[];
  start: Date;
  end: Date;
  allDay: boolean;
  status: CalendarItemStatus;
  source: string | null;
  /** Sidebar filter bucket: 'local' | 'caldav' | 'invites' | 'google:<id>'. */
  sourceKey: string;
  /** CSS color value for source tinting, e.g. `var(--accent-pink)`. */
  color: string | null;
}

/**
 * Hide an invitation only when the same account already exposes its UID as a
 * calendar event. UIDs are organizer-defined and therefore commonly appear in
 * multiple configured accounts for the same meeting.
 */
export function filterInvitesWithoutMirroredEvents(
  events: ReadonlyArray<Pick<BackendCalendarEvent, 'account_id' | 'uid'>>,
  invites: ReadonlyArray<BackendCalendarInvite>,
): BackendCalendarInvite[] {
  const eventKeys = new Set(events.map((event) => `${event.account_id}\0${event.uid}`));
  return invites.filter((invite) => !eventKeys.has(`${invite.account_id}\0${invite.uid}`));
}

const MINUTE_MS = 60_000;
const DEFAULT_DURATION_MS = 60 * MINUTE_MS;

export function itemFromEvent(event: BackendCalendarEvent): CalendarItem {
  const start = new Date(event.starts_at);
  let end = new Date(event.ends_at);
  if (!Number.isFinite(end.getTime()) || end.getTime() <= start.getTime()) {
    end = new Date(start.getTime() + DEFAULT_DURATION_MS);
  }
  const sourceKey =
    event.source === 'google' ? `google:${event.collection_href ?? 'primary'}` : event.source;
  return {
    sourceKey,
    color: null,
    key: `event:${event.id}`,
    kind: 'event',
    id: event.id,
    uid: event.uid,
    title: event.title.trim() || t('calendar.event.untitled'),
    location: event.location ?? null,
    description: descriptionToText(event.description),
    organizer: null,
    attendees: parseAttendees(event.attendees_json),
    start,
    end,
    allDay: event.all_day === 1,
    status: 'own',
    source: event.source,
  };
}

/** Invites without a start date cannot be placed on the grid → null. */
export function itemFromInvite(invite: BackendCalendarInvite): CalendarItem | null {
  if (!invite.starts_at) return null;
  const start = new Date(invite.starts_at);
  if (!Number.isFinite(start.getTime())) return null;
  let end = invite.ends_at ? new Date(invite.ends_at) : null;
  if (!end || !Number.isFinite(end.getTime()) || end.getTime() <= start.getTime()) {
    end = new Date(start.getTime() + DEFAULT_DURATION_MS);
  }
  let status: CalendarItemStatus = 'needs-action';
  if (invite.cancelled === 1) status = 'cancelled';
  else if (invite.rsvp_status === 'accepted') status = 'accepted';
  else if (invite.rsvp_status === 'declined') status = 'declined';
  else if (invite.rsvp_status === 'tentative') status = 'tentative';
  return {
    key: `invite:${invite.id}`,
    kind: 'invite',
    sourceKey: 'invites',
    color: null,
    id: invite.id,
    uid: invite.uid,
    title: invite.summary?.trim() || t('calendar.event.untitled'),
    location: invite.location ?? null,
    description: descriptionToText(invite.description),
    organizer: invite.organizer_name || invite.organizer_email || null,
    attendees: [],
    start,
    end,
    allDay: invite.all_day === 1,
    status,
    source: null,
  };
}

/**
 * Invite descriptions frequently arrive as HTML (Exchange, Zoom, GroupWise).
 * The calendar renders descriptions as plain text only, so markup is converted
 * instead of shown verbatim: line-break tags become newlines, remaining tags
 * are stripped, common entities are decoded. Plain-text input passes through
 * untouched — including literal `<` in prose.
 */
export function descriptionToText(raw: string | null | undefined): string | null {
  if (!raw) return null;
  if (!/<[a-z][^>]*>/i.test(raw)) return raw;
  const text = raw
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/(p|div|li|tr|h[1-6])>/gi, '\n')
    .replace(/<[^>]+>/g, '')
    .replace(/&nbsp;/gi, ' ')
    .replace(/&lt;/gi, '<')
    .replace(/&gt;/gi, '>')
    .replace(/&quot;/gi, '"')
    .replace(/&#39;|&apos;/gi, "'")
    // Last, so `&amp;lt;` decodes to the literal text `&lt;`, not to `<`.
    .replace(/&amp;/gi, '&')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
  return text || null;
}

function parseAttendees(raw: string): string[] {
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map((entry) => {
        if (typeof entry === 'string') return entry;
        if (entry && typeof entry === 'object' && 'email' in entry) {
          return String((entry as { email: unknown }).email);
        }
        return '';
      })
      .filter(Boolean);
  } catch {
    return [];
  }
}

// ---------------------------------------------------------------------------
// Calendar sources (sidebar filter)
// ---------------------------------------------------------------------------

export interface CalendarSource {
  key: string;
  label: string;
  color: string;
  count: number;
}

export interface CalendarSourceDefinition {
  key: string;
  label: string;
}

/** Google calendars cycle through these; local/caldav/invites have fixed colors. */
const GOOGLE_PALETTE = [
  'var(--accent-pink)',
  'var(--accent-orange)',
  'var(--accent-green)',
  'var(--accent-purple)',
  'var(--accent-amber)',
  'var(--accent-cyan)',
  'var(--accent-primary)',
];

/** Deterministic color per source key, stable across reloads. */
export function sourceColorMap(keys: Iterable<string>): Map<string, string> {
  const map = new Map<string, string>([
    ['local', 'var(--accent-primary)'],
    ['caldav', 'var(--accent-cyan)'],
    ['invites', 'var(--accent-purple)'],
  ]);
  const googleKeys = [...new Set([...keys].filter((key) => key.startsWith('google:')))].sort();
  googleKeys.forEach((key, index) => {
    map.set(key, GOOGLE_PALETTE[index % GOOGLE_PALETTE.length] ?? '');
  });
  return map;
}

export function sourceLabel(key: string): string {
  if (key === 'local') return t('calendar.source.local');
  if (key === 'caldav') return t('calendar.source.caldav');
  if (key === 'invites') return t('calendar.source.invites');
  return prettifyGoogleCalendarId(key.slice('google:'.length));
}

/**
 * Personal calendars are plain addresses and stay readable as they are.
 * Generated ids ("de.german#holiday@group.v.calendar.google.com") are noise —
 * keep the local part and turn its separators into spaces.
 */
function prettifyGoogleCalendarId(id: string): string {
  const at = id.indexOf('@');
  if (at === -1) return id === 'primary' ? t('calendar.source.googlePrimary') : id;
  const domain = id.slice(at + 1);
  if (!domain.endsWith('calendar.google.com')) return id;
  const cleaned = id
    .slice(0, at)
    .replace(/[#._-]+/g, ' ')
    .trim();
  return cleaned || id;
}

/** Distinct declared/event sources — Google calendars first, then the rest. */
export function collectSources(
  items: CalendarItem[],
  definitions: Iterable<CalendarSourceDefinition> = [],
): CalendarSource[] {
  const counts = new Map<string, number>();
  const labels = new Map<string, string>();
  for (const definition of definitions) {
    counts.set(definition.key, 0);
    labels.set(definition.key, definition.label);
  }
  for (const item of items) {
    counts.set(item.sourceKey, (counts.get(item.sourceKey) ?? 0) + 1);
  }
  const colors = sourceColorMap(counts.keys());
  const order = (key: string) => {
    if (key.startsWith('google:')) return 0;
    if (key === 'caldav') return 1;
    if (key === 'local') return 2;
    return 3;
  };
  return [...counts.keys()]
    .sort((a, b) => order(a) - order(b) || a.localeCompare(b))
    .map((key) => ({
      key,
      label: labels.get(key) ?? sourceLabel(key),
      color: colors.get(key) ?? 'var(--accent-primary)',
      count: counts.get(key) ?? 0,
    }));
}

// ---------------------------------------------------------------------------
// Date math
// ---------------------------------------------------------------------------

export function startOfDay(date: Date): Date {
  const copy = new Date(date);
  copy.setHours(0, 0, 0, 0);
  return copy;
}

export function addDays(date: Date, days: number): Date {
  const copy = new Date(date);
  copy.setDate(copy.getDate() + days);
  return copy;
}

export function addMonths(date: Date, months: number): Date {
  const copy = new Date(date);
  // Clamp to day 1 first so Jan 31 + 1 month does not skip February.
  copy.setDate(1);
  copy.setMonth(copy.getMonth() + months);
  return copy;
}

/** Monday-based start of week, matching German calendars. */
export function startOfWeek(date: Date): Date {
  const day = startOfDay(date);
  const weekday = (day.getDay() + 6) % 7; // Mon=0 … Sun=6
  return addDays(day, -weekday);
}

export function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** The seven days of the week containing `anchor`. */
export function weekDays(anchor: Date): Date[] {
  const monday = startOfWeek(anchor);
  return Array.from({ length: 7 }, (_, i) => addDays(monday, i));
}

/**
 * All grid days for the month containing `anchor`: starts on the Monday of the
 * first week, ends on the Sunday of the last — 35 or 42 days.
 */
export function monthGridDays(anchor: Date): Date[] {
  const first = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
  const gridStart = startOfWeek(first);
  const lastOfMonth = new Date(anchor.getFullYear(), anchor.getMonth() + 1, 0);
  const days: Date[] = [];
  let cursor = gridStart;
  while (cursor.getTime() <= lastOfMonth.getTime() || days.length % 7 !== 0) {
    days.push(cursor);
    cursor = addDays(cursor, 1);
  }
  return days;
}

/** True when the item covers any part of `day` ([00:00, 24:00) exclusive end). */
export function overlapsDay(item: CalendarItem, day: Date): boolean {
  const dayStart = startOfDay(day);
  const dayEnd = addDays(dayStart, 1);
  return item.start.getTime() < dayEnd.getTime() && item.end.getTime() > dayStart.getTime();
}

export function isMultiDay(item: CalendarItem): boolean {
  // An event ending exactly at midnight still belongs to its start day.
  const inclusiveEnd = new Date(item.end.getTime() - 1);
  return !isSameDay(item.start, inclusiveEnd);
}

/** Items touching `day`, all-day and multi-day first, then by start time. */
export function itemsForDay(items: CalendarItem[], day: Date): CalendarItem[] {
  return items
    .filter((item) => overlapsDay(item, day))
    .sort((a, b) => {
      const aBanner = a.allDay || isMultiDay(a) ? 0 : 1;
      const bBanner = b.allDay || isMultiDay(b) ? 0 : 1;
      if (aBanner !== bBanner) return aBanner - bBanner;
      return a.start.getTime() - b.start.getTime() || a.end.getTime() - b.end.getTime();
    });
}

// ---------------------------------------------------------------------------
// Week/day grid layout
// ---------------------------------------------------------------------------

export interface PositionedItem {
  item: CalendarItem;
  /** Minutes since 00:00, clamped to the day. */
  startMin: number;
  /** Display end; at least 30 minutes after `startMin`. */
  endMin: number;
  /** Number of already visible events this item overlaps at its start. */
  overlapLevel: number;
}

/** Minutes the item occupies within `day`, clamped to [0, 1440]. */
export function minutesInDay(item: CalendarItem, day: Date): { startMin: number; endMin: number } {
  const dayStart = startOfDay(day).getTime();
  const dayEnd = addDays(day, 1);
  dayEnd.setHours(0, 0, 0, 0);
  const start = Math.max(item.start.getTime(), dayStart);
  const end = Math.min(item.end.getTime(), dayEnd.getTime());
  return {
    startMin: Math.max(0, Math.floor((start - dayStart) / MINUTE_MS)),
    endMin: Math.min(1440, Math.ceil((end - dayStart) / MINUTE_MS)),
  };
}

/** Items rendered in the all-day lane of the week/day view. */
export function allDayItemsForDay(items: CalendarItem[], day: Date): CalendarItem[] {
  return itemsForDay(items, day).filter((item) => item.allDay || isMultiDay(item));
}

/**
 * Lays out the timed items of `day` in chronological stacking order. An item
 * that starts during another item is rendered above it with a small inset,
 * keeping almost the full day-column width available for its title.
 */
export function layoutDay(items: CalendarItem[], day: Date): PositionedItem[] {
  const timed = itemsForDay(items, day)
    .filter((item) => !item.allDay && !isMultiDay(item))
    .map((item) => {
      const { startMin, endMin } = minutesInDay(item, day);
      return {
        item,
        startMin,
        endMin: Math.max(endMin, startMin + 30),
        overlapLevel: 0,
      };
    })
    .sort((a, b) => a.startMin - b.startMin || b.endMin - a.endMin);

  let activeEnds: number[] = [];
  for (const entry of timed) {
    activeEnds = activeEnds.filter((endMin) => endMin > entry.startMin);
    entry.overlapLevel = activeEnds.length;
    activeEnds.push(entry.endMin);
  }
  return timed;
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/**
 * Weekday name of `day` in the UI locale. Every calendar header goes through
 * here, so the week grid cannot drift back to a hard-coded language while the
 * month grid follows the UI.
 */
export function formatWeekday(
  day: Date,
  locale: string,
  width: 'short' | 'narrow' = 'short',
): string {
  return new Intl.DateTimeFormat(locale, { weekday: width }).format(day);
}

/** Monday-first weekday names in the UI locale, for column headers. */
export function weekdayLabels(locale: string, width: 'short' | 'narrow' = 'short'): string[] {
  // 2024-01-01 is a Monday.
  return Array.from({ length: 7 }, (_, index) =>
    formatWeekday(new Date(2024, 0, 1 + index), locale, width),
  );
}

export function formatTime(date: Date): string {
  return d(date, 'time');
}

export function formatMonthYear(date: Date): string {
  return date.toLocaleDateString(currentLocale(), { month: 'long', year: 'numeric' });
}

export function formatDayLong(date: Date): string {
  return d(date, 'long');
}

export function formatWeekRange(anchor: Date): string {
  // weekDays always yields exactly the 7 days of the anchor's week.
  const [first, , , , , , last] = weekDays(anchor);
  if (!first || !last) {
    return '';
  }
  if (first.getMonth() === last.getMonth()) {
    return t('calendar.range.weekSameMonth', {
      first: first.getDate(),
      last: last.getDate(),
      monthYear: formatMonthYear(first),
    });
  }
  const firstStr = first.toLocaleDateString(currentLocale(), {
    day: 'numeric',
    month: 'short',
    year: first.getFullYear() === last.getFullYear() ? undefined : 'numeric',
  });
  const lastStr = last.toLocaleDateString(currentLocale(), {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
  });
  return `${firstStr} – ${lastStr}`;
}

export function formatItemRange(item: CalendarItem): string {
  const inclusiveEnd = new Date(item.end.getTime() - 1);
  if (item.allDay) {
    if (isSameDay(item.start, inclusiveEnd)) {
      return t('calendar.range.allDaySingle', { day: formatDayLong(item.start) });
    }
    const start = item.start.toLocaleDateString(currentLocale(), { day: 'numeric', month: 'long' });
    return t('calendar.range.allDayMulti', { start, end: formatDayLong(inclusiveEnd) });
  }
  if (isSameDay(item.start, inclusiveEnd)) {
    return t('calendar.range.sameDay', {
      day: formatDayLong(item.start),
      start: formatTime(item.start),
      end: formatTime(item.end),
    });
  }
  return t('calendar.range.span', {
    start: t('calendar.range.dayTime', {
      day: formatDayLong(item.start),
      time: formatTime(item.start),
    }),
    end: t('calendar.range.dayTime', {
      day: formatDayLong(item.end),
      time: formatTime(item.end),
    }),
  });
}

// ---------------------------------------------------------------------------
// Editor drafts
// ---------------------------------------------------------------------------

/** Form state for the create/edit dialog. */
export interface CalendarEventDraft {
  /** Set when editing an existing local event. */
  id: string | null;
  title: string;
  location: string;
  description: string;
  allDay: boolean;
  start: Date;
  end: Date;
  /** Comma/semicolon separated e-mail addresses. */
  attendees: string;
  sendInvites: boolean;
}

/** A fresh one-hour draft starting at `at`. */
export function emptyDraft(at: Date): CalendarEventDraft {
  return {
    id: null,
    title: '',
    location: '',
    description: '',
    allDay: false,
    start: new Date(at),
    end: new Date(at.getTime() + DEFAULT_DURATION_MS),
    attendees: '',
    sendInvites: false,
  };
}

export function draftFromItem(item: CalendarItem): CalendarEventDraft {
  return {
    id: item.kind === 'event' ? item.id : null,
    title: item.title,
    location: item.location ?? '',
    description: item.description ?? '',
    allDay: item.allDay,
    start: new Date(item.start),
    end: new Date(item.end),
    attendees: item.attendees.join(', '),
    sendInvites: false,
  };
}

/** Value for an `<input type="datetime-local">`, in local time. */
export function toDateTimeLocalInput(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** Value for an `<input type="date">`, in local time. */
export function toDateInput(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}
