import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { orderedLoad } from './internal/orderedLoad';
import { backendCommandsAvailable } from '@/services/backend/core';
import { tc } from '@/i18n/runtime';
import type { TaskSyncSummary } from '@/services/backend/generated/core/tasks/sync/TaskSyncSummary';
import {
  backendCreateTask,
  backendCreateTaskList,
  backendDeleteTask,
  backendDeleteTaskList,
  backendListTaskLists,
  backendListTasks,
  backendRenameTaskList,
  backendRestoreTask,
  backendSetTaskCompleted,
  backendSyncTasks,
  backendUndoTaskAdvance,
  backendUpdateTask,
  type BackendTask,
  type BackendTaskFields,
  type BackendTaskList,
} from '@/services/backend/tasks';

export type Task = BackendTask;
export type TaskList = BackendTaskList;
export type TaskFields = BackendTaskFields;

/** Which lists the view shows: one list, or every list at once. */
export const ALL_LISTS = 'all';

/** Opening the view syncs at most this often; Sync tasks always does. */
export const SYNC_ON_OPEN_MS = 5 * 60_000;

/**
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
export const useTasksStore = defineStore('tasks', () => {
  const lists = ref<TaskList[]>([]);
  const tasks = ref<Task[]>([]);
  const loader = orderedLoad();
  const { loading, loadFailed } = loader;
  const syncing = ref(false);
  /** The list new tasks go to and the view shows; persisted per session only. */
  const activeListId = ref<string>(ALL_LISTS);
  let lastSyncAt = 0;

  const defaultListId = computed(() =>
    lists.value.find((list) => list.provider === 'local' && list.name === '')?.id
      ?? lists.value.find((list) => !list.read_only)?.id
      ?? null,
  );

  /** Where a new task goes: the list on screen, or the default one. */
  const targetListId = computed(() => {
    const active = lists.value.find((list) => list.id === activeListId.value && !list.read_only);
    return active?.id ?? defaultListId.value;
  });

  async function load() {
    if (!backendCommandsAvailable()) return;
    await loader.load(
      () => Promise.all([backendListTaskLists(), backendListTasks(true)]),
      ([listRows, taskRows]) => {
        lists.value = listRows;
        tasks.value = taskRows;
        if (activeListId.value !== ALL_LISTS && !listRows.some((list) => list.id === activeListId.value)) {
          activeListId.value = ALL_LISTS;
        }
      },
    );
  }

  /** Push and pull remote lists, then reload. Returns the encoded errors. */
  async function sync(manual = false): Promise<string[]> {
    if (!backendCommandsAvailable() || syncing.value) return [];
    syncing.value = true;
    lastSyncAt = Date.now();
    try {
      const result = await backendSyncTasks({ manual });
      await load();
      return result.errors;
    } finally {
      syncing.value = false;
    }
  }

  /** Sync unless one ran in the last few minutes. */
  async function syncIfStale(): Promise<string[]> {
    if (Date.now() - lastSyncAt < SYNC_ON_OPEN_MS) return [];
    return sync();
  }

  function put(task: Task) {
    const index = tasks.value.findIndex((item) => item.id === task.id);
    if (index >= 0) tasks.value.splice(index, 1, task);
    else tasks.value.push(task);
  }

  async function refreshLists() {
    lists.value = await backendListTaskLists();
  }

  async function create(fields: TaskFields, options: { listId?: string | null; threadId?: string | null } = {}) {
    const listId = options.listId ?? targetListId.value;
    if (!listId) throw new Error('backend.errors.taskListNotFound');
    const task = await loader.mutate(() => backendCreateTask({ listId, fields, threadId: options.threadId }));
    put(task);
    void refreshLists();
    return task;
  }

  async function update(task: Task, fields: TaskFields) {
    const saved = await loader.mutate(() =>
      backendUpdateTask({ id: task.id, expectedUpdatedAt: task.updated_at, fields }));
    put(saved);
    void refreshLists();
    return saved;
  }

  async function setCompleted(task: Task, completed: boolean) {
    const before = { ...task };
    // A simple repeating task moves to its next date instead; showing it
    // completed first would make the row jump out and back.
    if (task.recurrence !== 'simple' || !completed) {
      put({ ...task, completed_at: completed ? new Date().toISOString() : null });
    }
    try {
      const saved = await loader.mutate(() => backendSetTaskCompleted(task.id, completed));
      put(saved);
      void refreshLists();
      return saved;
    } catch (error) {
      put(before);
      throw error;
    }
  }

  /** Undo of completing a repeating task. */
  async function undoAdvance(task: Task) {
    const restored = await loader.mutate(() => backendUndoTaskAdvance(task.id));
    put(restored);
    return restored;
  }

  async function remove(task: Task) {
    await loader.mutate(() => backendDeleteTask(task.id));
    tasks.value = tasks.value.filter((item) => item.id !== task.id);
    void refreshLists();
  }

  /**
   * Undo a delete. A local task is gone for good once deleted, so undo
   * creates it again; a remote one whose deletion has not been pushed yet is
   * restored in place.
   */
  async function restore(task: Task) {
    const list = lists.value.find((item) => item.id === task.list_id);
    let restored: Task;
    if (list && list.provider !== 'local') {
      try {
        restored = await loader.mutate(() => backendRestoreTask(task.id));
      } catch {
        restored = await recreate(task);
      }
    } else {
      restored = await recreate(task);
    }
    put(restored);
    void refreshLists();
    return restored;
  }

  function recreate(task: Task) {
    return loader.mutate(() => backendCreateTask({
      listId: task.list_id,
      threadId: task.thread_id,
      fields: fieldsOf(task),
    }));
  }

  async function createList(name: string) {
    const list = await backendCreateTaskList(name);
    await refreshLists();
    activeListId.value = list.id;
    return list;
  }

  async function renameList(id: string, name: string) {
    await backendRenameTaskList(id, name);
    await refreshLists();
  }

  async function deleteList(id: string) {
    await loader.mutate(() => backendDeleteTaskList(id));
    if (activeListId.value === id) activeListId.value = ALL_LISTS;
    await load();
  }

  function find(id: string): Task | undefined {
    return tasks.value.find((task) => task.id === id);
  }

  function list(id: string): TaskList | undefined {
    return lists.value.find((item) => item.id === id);
  }

  return {
    lists,
    tasks,
    loading,
    loadFailed,
    syncing,
    activeListId,
    defaultListId,
    targetListId,
    load,
    sync,
    syncIfStale,
    create,
    update,
    setCompleted,
    undoAdvance,
    remove,
    restore,
    createList,
    renameList,
    deleteList,
    find,
    list,
  };
});

/** The editable fields of a stored task. */
export function fieldsOf(task: Task): TaskFields {
  return {
    title: task.title,
    notes: task.notes,
    due_at: task.due_at,
    reminder_at: task.reminder_at,
    priority: task.priority,
    completed_at: task.completed_at,
  };
}
