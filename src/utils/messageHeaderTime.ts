import { t } from '@/i18n/runtime';
import {
  DATE_WITH_YEAR_OPTIONS,
  TIME_OPTIONS,
  formatMessageTimestamp,
  localDayNumber,
  localeFormatter,
} from './listTime';
// Its own module: only the reader uses it, and listTime.ts is in the entry chunk.
/**
 * Compact time for a message header, the way phone mail apps write it:
 * "08:30" today, "Yesterday, 08:30", "27 Oct, 08:30" this year, then with the
 * year. The date only appears when it says something.
 */
export function formatMessageHeaderTime(value: string): string {
  const date = new Date(value);
  const now = new Date();
  const diffDays = localDayNumber(now) - localDayNumber(date);
  const time = localeFormatter('time', TIME_OPTIONS).format(date);
  if (diffDays === 0) return time;
  if (diffDays === 1) return `${t('common.yesterday')}, ${time}`;
  if (date.getFullYear() === now.getFullYear()) return formatMessageTimestamp(value);
  return `${localeFormatter('dateWithYear', DATE_WITH_YEAR_OPTIONS).format(date)}, ${time}`;
}
