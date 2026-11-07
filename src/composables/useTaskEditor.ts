import { reactive } from 'vue';

export interface TaskEditorRequest {
  /** Edit this task; absent for a new one. */
  taskId?: string;
  /** A new task made from this thread, linked back to it. */
  threadId?: string;
  /** Prefilled title. */
  title?: string;
  /**
   * The thread's subject; the editor strips Re:/Fwd: from it. Done there, not
   * here: the prefix tables come with the composer's text helpers, which have
   * no business in the startup bundle this file is part of.
   */
  subject?: string;
  listId?: string;
}

const state = reactive({ request: null as TaskEditorRequest | null });

/**
 * The task editor, opened from anywhere: the Tasks view, the reader's menu,
 * the palette and the `T` key on a thread. `TaskEditorDialog` is mounted once
 * in `App.vue` and loads the tasks store when it opens, so nothing of tasks
 * is parsed at startup.
 */
export function useTaskEditor() {
  function openNew(request: Omit<TaskEditorRequest, 'taskId'> = {}) {
    state.request = { ...request };
  }

  function openForThread(threadId: string, subject: string | null | undefined) {
    if (!threadId) return false;
    state.request = { threadId, subject: subject ?? '' };
    return true;
  }

  function openTask(taskId: string) {
    state.request = { taskId };
  }

  function close() {
    state.request = null;
  }

  return { state, openNew, openForThread, openTask, close };
}
