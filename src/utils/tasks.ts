import { parseReminderDate } from '@/utils/reminderDate';

/** The parts of a task these helpers read; the wire record has more. */
export interface TaskLike {
  id: string;
  title: string;
  due_at: string | null;
  priority: number;
  completed_at: string | null;
  created_at: string;
}

export type TaskDueState = 'overdue' | 'today' | 'upcoming' | 'none';
export type TaskGroup = 'overdue' | 'today' | 'upcoming' | 'someday' | 'completed';

/** Priority on the iCalendar scale (1 highest, 9 lowest), in three steps. */
export type TaskPriorityLevel = 'none' | 'high' | 'medium' | 'low';

export function priorityLevel(priority: number): TaskPriorityLevel {
  if (priority <= 0) return 'none';
  if (priority <= 4) return 'high';
  if (priority === 5) return 'medium';
  return 'low';
}

export const PRIORITY_VALUES: Record<TaskPriorityLevel, number> = { none: 0, high: 1, medium: 5, low: 9 };

/** `YYYY-MM-DD` of a local date. */
export function localDateKey(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${date.getFullYear()}-${month}-${day}`;
}

export function isDateOnly(due: string): boolean {
  return /^\d{4}-\d{2}-\d{2}$/.test(due);
}

/** The local calendar day a due value falls on. */
export function dueDay(due: string): string {
  return isDateOnly(due) ? due : localDateKey(new Date(due));
}

export function dueState(task: Pick<TaskLike, 'due_at' | 'completed_at'>, now = new Date()): TaskDueState {
  if (!task.due_at) return 'none';
  const today = localDateKey(now);
  const day = dueDay(task.due_at);
  if (day < today) return 'overdue';
  if (day > today) return 'upcoming';
  // A time today that has passed is overdue; a date today is not.
  if (!isDateOnly(task.due_at) && new Date(task.due_at).getTime() < now.getTime()) return 'overdue';
  return 'today';
}

function sortInstant(due: string | null): number {
  if (!due) return Number.POSITIVE_INFINITY;
  if (isDateOnly(due)) {
    const [year, month, day] = due.split('-').map(Number);
    // A date sorts after the timed tasks of the same day.
    return new Date(year!, month! - 1, day!, 23, 59, 59, 999).getTime();
  }
  return new Date(due).getTime();
}

/** Due first (soonest first), then by priority, then oldest first. */
export function compareTasks(left: TaskLike, right: TaskLike): number {
  const due = sortInstant(left.due_at) - sortInstant(right.due_at);
  if (due !== 0 && Number.isFinite(due)) return due;
  if (left.due_at && !right.due_at) return -1;
  if (!left.due_at && right.due_at) return 1;
  const leftPriority = left.priority > 0 ? left.priority : 10;
  const rightPriority = right.priority > 0 ? right.priority : 10;
  if (leftPriority !== rightPriority) return leftPriority - rightPriority;
  return left.created_at.localeCompare(right.created_at);
}

export function taskGroup(task: TaskLike, now = new Date()): TaskGroup {
  if (task.completed_at) return 'completed';
  const state = dueState(task, now);
  return state === 'none' ? 'someday' : state;
}

const GROUP_ORDER: readonly TaskGroup[] = ['overdue', 'today', 'upcoming', 'someday', 'completed'];

/** Tasks in display order, grouped; completed ones newest first. */
export function groupTasks<T extends TaskLike>(tasks: readonly T[], now = new Date()): Array<{ group: TaskGroup; tasks: T[] }> {
  const buckets = new Map<TaskGroup, T[]>();
  for (const task of tasks) {
    const group = taskGroup(task, now);
    const bucket = buckets.get(group) ?? [];
    bucket.push(task);
    buckets.set(group, bucket);
  }
  return GROUP_ORDER.flatMap((group) => {
    const bucket = buckets.get(group);
    if (!bucket?.length) return [];
    const sorted = group === 'completed'
      ? [...bucket].sort((a, b) => (b.completed_at ?? '').localeCompare(a.completed_at ?? ''))
      : [...bucket].sort(compareTasks);
    return [{ group, tasks: sorted }];
  });
}

/** What a due value reads as in a row: "Today 15:00", "Tomorrow", "Fri 14 Nov". */
export function formatDue(
  due: string,
  locale: string,
  labels: { today: string; tomorrow: string; yesterday: string },
  now = new Date(),
): string {
  const day = dueDay(due);
  const today = localDateKey(now);
  const tomorrow = localDateKey(new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1));
  const yesterday = localDateKey(new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1));
  const [year, month, date] = day.split('-').map(Number);
  const local = new Date(year!, month! - 1, date!);
  let label: string;
  if (day === today) label = labels.today;
  else if (day === tomorrow) label = labels.tomorrow;
  else if (day === yesterday) label = labels.yesterday;
  else {
    label = new Intl.DateTimeFormat(locale, {
      weekday: 'short',
      day: 'numeric',
      month: 'short',
      ...(year === now.getFullYear() ? {} : { year: 'numeric' }),
    }).format(local);
  }
  if (isDateOnly(due)) return label;
  const time = new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' }).format(new Date(due));
  return `${label} ${time}`;
}

/* ── Quick add ─────────────────────────────────────────────────────────── */

/** Words a trailing date phrase may start with. Two-letter weekdays are left out: "do", "so", "am" are words. */
const LEADS = new Set([
  'today', 'tonight', 'tomorrow', 'heute', 'morgen', 'uebermorgen',
  'next', 'naechste', 'naechsten', 'in', 'at', 'um',
  'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday',
  'mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun',
  'montag', 'dienstag', 'mittwoch', 'donnerstag', 'freitag', 'samstag', 'sonntag',
]);

/** Words that join the title to the date and go with it: "Pay rent by Friday". */
const CONNECTORS = new Set(['by', 'on', 'due', 'until', 'bis', 'am']);

/** The German leads with umlauts, as the reminder parser spells them. */
const UMLAUT_LEADS = /^(übermorgen|nächste[nr]?)$/;

/** A date or a time that cannot be a count: "24.12.", "2026-12-24", "15:00", "3pm". */
const DATE_TOKEN = /^(\d{1,2}\.\d{1,2}\.(\d{2,4})?|20\d{2}-\d{1,2}-\d{1,2}|\d{1,2}:\d{2}|\d{1,2}(am|pm))$/i;
/** Every other word a date phrase may contain after its first one. */
const PHRASE_WORDS = new Set([
  'week', 'weeks', 'woche', 'wochen', 'month', 'monat', 'day', 'days', 'tag', 'tage', 'tagen',
  'hour', 'hours', 'h', 'stunde', 'stunden', 'min', 'mins', 'minute', 'minutes', 'minuten',
  'after', 'am', 'pm', 'uhr', 'morning', 'afternoon', 'evening', 'vormittag', 'nachmittag',
  'abend', 'morgens', 'abends',
]);
const NUMBER = /^\d{1,4}$/;
const TIME_HINT = /\d{1,2}:\d{2}|\d\s*(am|pm|uhr)\b|\b(at|um)\s+\d|\b(tonight|morning|afternoon|evening|vormittag|nachmittag|abend|morgens)\b|\bin\s+\d+\s*(min|minute|minuten|h|hour|hours|stunde|stunden)\b/i;

export interface QuickAdd {
  title: string;
  /** `YYYY-MM-DD`, an ISO instant when a time was named, or null. */
  dueAt: string | null;
}

function leads(word: string): boolean {
  const lower = word.toLocaleLowerCase();
  return LEADS.has(lower) || UMLAUT_LEADS.test(lower) || DATE_TOKEN.test(lower);
}

/** The whole phrase reads as a date: "Monday about the contract" does not. */
function datePhrase(words: readonly string[]): boolean {
  return leads(words[0]!) && words.slice(1).every((word) => {
    const lower = word.toLocaleLowerCase();
    return leads(word) || PHRASE_WORDS.has(lower) || NUMBER.test(lower);
  });
}

/**
 * Split "Call Anna tomorrow 15:00" into a title and a due value, using the
 * reminder parser the snooze picker already understands (English and German).
 * Only a trailing phrase that starts like a date counts, so "Call 3 people"
 * stays a title.
 */
export function parseQuickAdd(input: string, now = new Date()): QuickAdd {
  const words = input.trim().split(/\s+/).filter(Boolean);
  for (let take = Math.min(6, words.length - 1); take >= 1; take -= 1) {
    const phrase = words.slice(words.length - take);
    if (!datePhrase(phrase)) continue;
    const text = phrase.join(' ');
    let titleWords = words.slice(0, words.length - take);
    while (titleWords.length > 1 && CONNECTORS.has(titleWords.at(-1)!.toLocaleLowerCase())) {
      titleWords = titleWords.slice(0, -1);
    }
    const title = titleWords.join(' ');
    if (/^(today|heute)$/i.test(text)) return { title, dueAt: localDateKey(now) };
    const date = parseReminderDate(text, now);
    if (!date) continue;
    return { title, dueAt: TIME_HINT.test(text) ? date.toISOString() : localDateKey(date) };
  }
  return { title: input.trim(), dueAt: null };
}
