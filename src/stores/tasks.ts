import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { orderedLoad } from './internal/orderedLoad';
import { backendCommandsAvailable } from '@/services/backend/core';
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
  async function sync(): Promise<string[]> {
    if (!backendCommandsAvailable() || syncing.value) return [];
    syncing.value = true;
    try {
      const result = await backendSyncTasks();
      await load();
      return result.errors;
    } finally {
      syncing.value = false;
    }
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
    put({ ...task, completed_at: completed ? new Date().toISOString() : null });
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
    create,
    update,
    setCompleted,
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
