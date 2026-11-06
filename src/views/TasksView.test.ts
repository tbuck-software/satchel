// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendTask, BackendTaskList } from '@/services/backend/tasks';
import { useTaskEditor } from '@/composables/useTaskEditor';
import { useToast } from '@/composables/useToast';
import { localDateKey } from '@/utils/tasks';
import { useUiStore } from '@/stores/ui';
import { answerConfirms } from '@/test/dialogs';
import TasksView from './TasksView.vue';

const harness = vi.hoisted(() => ({
  push: vi.fn(),
  lists: [] as BackendTaskList[],
  tasks: [] as BackendTask[],
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: harness.push }),
  useRoute: () => ({ query: {} }),
}));

vi.mock('@/services/backend/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend/core')>()),
  backendCommandsAvailable: () => true,
}));

vi.mock('@/services/backend/tasks', () => ({
  backendListTaskLists: vi.fn(async () => harness.lists),
  backendListTasks: vi.fn(async () => harness.tasks),
  backendSyncTasks: vi.fn(async () => ({ summary: { lists: 0, pulled: 0, pushed: 0, removed: 0, conflicts: 0 }, errors: [] })),
  backendCreateTask: vi.fn(async (input: { listId: string; fields: { title: string; due_at: string | null } }) =>
    task(`created-${input.fields.title}`, { list_id: input.listId, title: input.fields.title, due_at: input.fields.due_at })),
  backendUpdateTask: vi.fn(),
  backendSetTaskCompleted: vi.fn(async (id: string, completed: boolean) => {
    const found = harness.tasks.find((item) => item.id === id)!;
    // As the backend does for a simple series: the next occurrence, still open.
    if (completed && found.recurrence === 'simple') return { ...found, due_at: '2030-01-08' };
    return { ...found, completed_at: completed ? '2026-11-06T10:00:00Z' : null };
  }),
  backendDeleteTask: vi.fn(async () => undefined),
  backendRestoreTask: vi.fn(),
  backendCreateTaskList: vi.fn(),
  backendRenameTaskList: vi.fn(),
  backendDeleteTaskList: vi.fn(),
}));

import * as tasksBackend from '@/services/backend/tasks';

function list(id: string, extra: Partial<BackendTaskList> = {}): BackendTaskList {
  return { id, account_id: null, provider: 'local', name: '', color: null, read_only: false, gone: false, last_synced_at: null, open_count: 0, ...extra };
}

function task(id: string, extra: Partial<BackendTask> = {}): BackendTask {
  return {
    id, list_id: 'local', title: id, notes: null, due_at: null, reminder_at: null, priority: 0,
    completed_at: null, recurrence: null, thread_id: null, message_id: null, pending: false,
    created_at: '2026-11-01T00:00:00Z', updated_at: '2026-11-01T00:00:00Z', ...extra,
  };
}

function press(key: string) {
  window.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
}

const mounted: Array<ReturnType<typeof mount>> = [];

async function mountView() {
  const wrapper = mount(TasksView, { attachTo: document.body });
  mounted.push(wrapper);
  await flushPromises();
  return wrapper;
}

describe('tasks view', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    const yesterday = new Date();
    yesterday.setDate(yesterday.getDate() - 1);
    harness.lists = [list('local')];
    harness.tasks = [
      task('someday'),
      task('late', { due_at: localDateKey(yesterday), thread_id: 'th-1' }),
      task('done', { completed_at: '2026-11-01T00:00:00Z' }),
    ];
  });

  afterEach(() => {
    while (mounted.length) mounted.pop()!.unmount();
    useToast().state.toasts.splice(0);
    useTaskEditor().close();
  });

  it('groups open tasks by when they are due and hides completed ones behind a toggle', async () => {
    const wrapper = await mountView();
    const titles = wrapper.findAll('.task-row__title').map((node) => node.text());
    expect(titles).toEqual(['late', 'someday']);
    expect(wrapper.text()).toContain('Overdue');

    const toggle = wrapper.findAll('button').find((button) => button.text().includes('completed'));
    await toggle!.trigger('click');
    expect(wrapper.findAll('.task-row__title').map((node) => node.text())).toEqual(['late', 'someday', 'done']);
  });

  it('completes, opens and deletes the focused task with the mail keys', async () => {
    await mountView();
    press('x');
    await flushPromises();
    expect(tasksBackend.backendSetTaskCompleted).toHaveBeenCalledWith('late', true);
    expect(useToast().state.toasts.at(-1)?.action?.undo).toBe(true);

    press('Enter');
    expect(useTaskEditor().state.request).toEqual({ taskId: 'someday' });
    useTaskEditor().close();

    press('#');
    await flushPromises();
    expect(tasksBackend.backendDeleteTask).toHaveBeenCalledWith('someday');
  });

  it('leaves the second key of a g sequence to the navigation', async () => {
    await mountView();
    useUiStore().showKeySequenceHint('D Drafts');
    for (const key of ['d', 'e', 'x', '#']) press(key);
    await flushPromises();
    expect(tasksBackend.backendSetTaskCompleted).not.toHaveBeenCalled();
    expect(tasksBackend.backendDeleteTask).not.toHaveBeenCalled();
    useUiStore().clearKeySequenceHint();
  });

  it('moves a simple repeating task on and asks before ending a complex series', async () => {
    harness.tasks = [
      task('weekly', { due_at: '2030-01-01', recurrence: 'simple' }),
      task('monthly', { due_at: '2030-01-02', recurrence: 'complex' }),
    ];
    const confirms = answerConfirms(false);
    const wrapper = await mountView();
    expect(wrapper.findAll('[data-testid="task-repeats"]')).toHaveLength(2);

    press('x');
    await flushPromises();
    expect(tasksBackend.backendSetTaskCompleted).toHaveBeenCalledWith('weekly', true);
    expect(confirms.asked).not.toHaveBeenCalled();
    expect(useToast().state.toasts.at(-1)?.message).toContain('Next: “weekly”');

    // The weekly task moved to 8 January, so the monthly one is first now.
    press('x');
    await flushPromises();
    expect(confirms.asked).toHaveBeenCalledWith(expect.objectContaining({
      body: 'This task repeats. Completing it here ends the series for all your devices.',
      confirmLabel: 'Complete series',
    }));
    expect(tasksBackend.backendSetTaskCompleted).not.toHaveBeenCalledWith('monthly', true);

    confirms.reply(true);
    press('x');
    await flushPromises();
    expect(tasksBackend.backendSetTaskCompleted).toHaveBeenCalledWith('monthly', true);
    confirms.stop();
  });

  it('takes no edits in a read-only list', async () => {
    harness.lists = [list('shared', { provider: 'caldav', account_id: 'acc', read_only: true })];
    harness.tasks = [task('theirs', { list_id: 'shared' })];
    const wrapper = await mountView();
    expect(wrapper.get('[data-testid="task-check"]').attributes('disabled')).toBeDefined();
    press('x');
    press('#');
    await flushPromises();
    expect(tasksBackend.backendSetTaskCompleted).not.toHaveBeenCalled();
    expect(tasksBackend.backendDeleteTask).not.toHaveBeenCalled();
  });

  it('links back to the mail a task came from', async () => {
    const wrapper = await mountView();
    await wrapper.get('.task-row__mail').trigger('click');
    expect(harness.push).toHaveBeenCalledWith({ name: 'thread', params: { id: 'th-1' } });
  });

  it('quick-adds with a natural due date into the default list', async () => {
    const wrapper = await mountView();
    const input = wrapper.get('[data-testid="task-quick-add"]');
    await input.setValue('Pay rent tomorrow');
    expect(wrapper.text()).toContain('Tomorrow');
    await wrapper.get('form.quick-add').trigger('submit');
    await flushPromises();
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    expect(tasksBackend.backendCreateTask).toHaveBeenCalledWith(expect.objectContaining({
      listId: 'local',
      fields: expect.objectContaining({ title: 'Pay rent', due_at: localDateKey(tomorrow) }),
    }));
    expect((input.element as HTMLInputElement).value).toBe('');
  });
});
