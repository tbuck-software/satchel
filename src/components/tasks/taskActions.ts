import { t } from '@/i18n/runtime';
import { useToast } from '@/composables/useToast';
import { useTasksStore, type Task } from '@/stores/tasks';
import { backendErrorLabel } from '@/utils/backendErrorLabels';

/**
 * The two task actions people repeat down a list, with the receipt the mail
 * actions give: a toast whose Undo is also `z` and the palette's Undo.
 */
export function useTaskActions() {
  const tasks = useTasksStore();
  const toast = useToast();

  async function toggleCompleted(task: Task): Promise<boolean> {
    const completing = !task.completed_at;
    try {
      const saved = await tasks.setCompleted(task, completing);
      if (completing) {
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
