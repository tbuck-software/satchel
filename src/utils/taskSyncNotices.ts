import { tc } from '@/i18n/runtime';
import type { TaskSyncSummary } from '@/services/backend/generated/core/tasks/sync/TaskSyncSummary';

/**
 * Loaded only when a sync reports something, so these sentences stay out of
 * the catalogue the Today card brings at first paint.
 *
 * What a sync changed behind the person's back, as one sentence each: the
 * server's version replacing an edit, a deleted task coming back, a list
 * that turned out read-only. Silence here would make tasks change unexplained.
 */
export function syncNotices(summary: TaskSyncSummary): string[] {
  const notices: string[] = [];
  const add = (count: number, key: string) => {
    if (count > 0) notices.push(tc(key, count, { count }));
  };
  add(summary.conflicts, 'tasks.notices.conflicts');
  add(summary.restored, 'tasks.notices.restored');
  add(summary.read_only_dropped, 'tasks.notices.readOnlyDropped');
  add(summary.read_only_kept, 'tasks.notices.readOnlyKept');
  add(summary.kept_local, 'tasks.notices.keptLocal');
  add(summary.lists_gone, 'tasks.notices.listsGone');
  return notices;
}

/**
 * Tasks and their lists. Loaded with the Tasks view, the editor and the
 * sidebar's Today card — never at startup.
 *
 * Writes go to the backend first and replace the row with what it returns;
 * completion is optimistic because it is the one action people repeat
 * quickly down a list.
 */
