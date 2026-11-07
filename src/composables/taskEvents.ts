import { listenToBackend } from '@/services/appEvents';
import { backendSupportsNativeCommands } from '@/services/backend/native';
import type { TaskSyncSummary } from '@/services/backend/generated/core/tasks/sync/TaskSyncSummary';
import { useToast } from '@/composables/useToast';
import { useTasksStore } from '@/stores/tasks';

let subscribed = false;

/**
 * Every task sync (the view's, the periodic groupware sync, a background push
 * after an edit) announces what it changed as `tasks:changed`: the store
 * reloads, so the view and the Today card follow, and what the person should
 * know (the server's version won, a deleted task came back, a list is
 * read-only) is said once. Subscribed by whichever surface loads tasks first;
 * it stays for the session, like the store.
 */
export function followTaskSyncs() {
  if (subscribed || !backendSupportsNativeCommands()) return;
  subscribed = true;
  void listenToBackend<TaskSyncSummary>('tasks:changed', (summary) => {
    void useTasksStore().load().catch(() => undefined);
    void import('@/utils/taskSyncNotices').then(({ syncNotices }) => {
      const toast = useToast();
      for (const notice of syncNotices(summary)) toast.info(notice);
    });
  }).catch(() => {
    subscribed = false;
  });
}
