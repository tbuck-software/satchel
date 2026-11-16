import type { TaskListRecord as WireTaskList } from './generated/core/db/tasks/TaskListRecord';
import type { TaskRecord as WireTask } from './generated/core/db/tasks/TaskRecord';
import type { TaskFieldsInput } from './generated/app/commands/tasks/TaskFieldsInput';
import type { TasksSyncResult } from './generated/app/commands/tasks/TasksSyncResult';
import { invoke } from './core';

/** A task list: local (no account) or a provider collection (CalDAV VTODO). */
export type BackendTaskList = WireTaskList;
/** One task. `due_at` is `YYYY-MM-DD` for a date, an RFC 3339 instant for a time. */
export type BackendTask = WireTask;
/** The editable fields of a task. */
export type BackendTaskFields = TaskFieldsInput;
export type BackendTasksSyncResult = TasksSyncResult;

/** Every list; a default local list exists even without accounts. */
export async function backendListTaskLists(): Promise<BackendTaskList[]> {
  return invoke<BackendTaskList[]>('list_task_lists');
}

/** Open tasks, and the most recent completed ones when asked for. */
export async function backendListTasks(includeCompleted = false): Promise<BackendTask[]> {
  return invoke<BackendTask[]>('list_tasks', { includeCompleted });
}

/** A new task; with `threadId` it links back to that mail. */
export async function backendCreateTask(input: {
  listId: string;
  fields: BackendTaskFields;
  threadId?: string | null;
}): Promise<BackendTask> {
  return invoke<BackendTask>('create_task', {
    input: { list_id: input.listId, fields: input.fields, thread_id: input.threadId ?? null },
  });
}

export async function backendUpdateTask(input: {
  id: string;
  expectedUpdatedAt?: string | null;
  fields: BackendTaskFields;
}): Promise<BackendTask> {
  return invoke<BackendTask>('update_task', {
    input: { id: input.id, expected_updated_at: input.expectedUpdatedAt ?? null, fields: input.fields },
  });
}

export async function backendSetTaskCompleted(id: string, completed: boolean): Promise<BackendTask> {
  return invoke<BackendTask>('set_task_completed', { id, completed });
}

export async function backendDeleteTask(id: string): Promise<void> {
  return invoke<void>('delete_task', { id });
}

/** Undo a deletion the server has not been told about yet. */
export async function backendRestoreTask(id: string): Promise<BackendTask> {
  return invoke<BackendTask>('restore_task', { id });
}

export async function backendCreateTaskList(name: string): Promise<BackendTaskList> {
  return invoke<BackendTaskList>('create_task_list', { name });
}

export async function backendRenameTaskList(id: string, name: string): Promise<BackendTaskList> {
  return invoke<BackendTaskList>('rename_task_list', { id, name });
}

export async function backendDeleteTaskList(id: string): Promise<void> {
  return invoke<void>('delete_task_list', { id });
}

/**
 * Push local changes and pull remote ones for every account with task lists.
 * `manual` (the Sync tasks button) also retries tasks that are backing off and
 * lists a refused write made read-only.
 */
export async function backendSyncTasks(options: { accountId?: string; manual?: boolean } = {}): Promise<BackendTasksSyncResult> {
  return invoke<BackendTasksSyncResult>('sync_tasks', {
    accountId: options.accountId ?? null,
    manual: options.manual ?? false,
  });
}

/** Undo completing a repeating task: back to the occurrence it moved on from. */
export async function backendUndoTaskAdvance(id: string): Promise<BackendTask> {
  return invoke<BackendTask>('undo_task_advance', { id });
}
