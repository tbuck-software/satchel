import { currentLocale, t } from '@/i18n/runtime';
import { useConfirm } from '@/composables/useConfirm';
import { useToast } from '@/composables/useToast';
import { useTasksStore, type Task } from '@/stores/tasks';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { formatDue } from '@/utils/tasks';

/** The due date as rows show it, for receipts. */
export function dueReceipt(due: string): string {
  return formatDue(due, currentLocale(), {
    today: t('tasks.due.today'),
    tomorrow: t('tasks.due.tomorrow'),
    yesterday: t('tasks.due.yesterday'),
  });
}

/**
 * Completing a series Satchel cannot step itself ends it on every device that
 * syncs the list; that needs a yes first. Simple rules move on instead.
 */
export async function confirmSeriesEnd(task: Pick<Task, 'recurrence'>): Promise<boolean> {
  if (task.recurrence !== 'complex') return true;
  return useConfirm().ask({
    title: t('tasks.recurring.confirmTitle'),
    body: t('tasks.recurring.confirmBody'),
    confirmLabel: t('tasks.recurring.completeSeries'),
  });
}

/**
 * The two task actions people repeat down a list, with the receipt the mail
 * actions give: a toast whose Undo is also `z` and the palette's Undo.
 */
export function useTaskActions() {
  const tasks = useTasksStore();
  const toast = useToast();

  async function toggleCompleted(task: Task): Promise<boolean> {
    const completing = !task.completed_at;
    if (completing && !(await confirmSeriesEnd(task))) return false;
    try {
      const saved = await tasks.setCompleted(task, completing);
      if (completing && !saved.completed_at && saved.due_at) {
        // A repeating task moved on to its next occurrence.
        toast.info(t('tasks.toast.nextOccurrence', { title: task.title, due: dueReceipt(saved.due_at) }), {
          action: { label: t('common.undo'), undo: true, handler: () => tasks.undoAdvance(saved) },
        });
      } else if (completing) {
        toast.info(t('tasks.toast.completed', { title: task.title }), {
          action: { label: t('common.undo'), undo: true, handler: () => tasks.setCompleted(saved, false) },
        });
      }
      return true;
    } catch (error) {
      toast.error(backendErrorLabel(error));
      return false;
    }
  }

  async function remove(task: Task): Promise<boolean> {
    try {
      await tasks.remove(task);
      toast.info(t('tasks.toast.deleted'), {
        action: { label: t('common.undo'), undo: true, handler: () => tasks.restore(task) },
      });
      return true;
    } catch (error) {
      toast.error(backendErrorLabel(error));
      return false;
    }
  }

  return { toggleCompleted, remove };
}
