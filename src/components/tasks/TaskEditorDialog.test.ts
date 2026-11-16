// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendTask, BackendTaskList } from '@/services/backend/tasks';
import { useTaskEditor } from '@/composables/useTaskEditor';
import { useToast } from '@/composables/useToast';
import TaskEditorDialog from './TaskEditorDialog.vue';

const harness = vi.hoisted(() => ({
  push: vi.fn(),
  lists: [] as BackendTaskList[],
  tasks: [] as BackendTask[],
}));

vi.mock('vue-router', () => ({ useRouter: () => ({ push: harness.push }) }));

vi.mock('@/services/backend/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend/core')>()),
  backendCommandsAvailable: () => true,
}));

vi.mock('@/services/backend/tasks', () => ({
  backendListTaskLists: vi.fn(async () => harness.lists),
  backendListTasks: vi.fn(async () => harness.tasks),
  backendCreateTask: vi.fn(async (input: { listId: string; threadId: string | null; fields: { title: string } }) =>
    task('new', { list_id: input.listId, title: input.fields.title, thread_id: input.threadId })),
  backendUpdateTask: vi.fn(async (input: { id: string; fields: Partial<BackendTask> }) =>
    task(input.id, { ...harness.tasks.find((item) => item.id === input.id), ...input.fields })),
  backendSetTaskCompleted: vi.fn(),
  backendDeleteTask: vi.fn(async () => undefined),
  backendRestoreTask: vi.fn(),
  backendUndoTaskAdvance: vi.fn(),
  backendSyncTasks: vi.fn(),
  backendCreateTaskList: vi.fn(),
  backendRenameTaskList: vi.fn(),
  backendDeleteTaskList: vi.fn(),
}));

import * as tasksBackend from '@/services/backend/tasks';

function task(id: string, extra: Partial<BackendTask> = {}): BackendTask {
  return {
    id, list_id: 'local', title: id, notes: null, due_at: null, reminder_at: null, priority: 0,
    completed_at: null, recurrence: null, thread_id: null, message_id: null, pending: false,
    created_at: '2026-11-01T00:00:00Z', updated_at: '2026-11-01T00:00:00Z', ...extra,
  };
}

const mounted: Array<ReturnType<typeof mount>> = [];

async function mountDialog() {
  const wrapper = mount(TaskEditorDialog, { attachTo: document.body });
  mounted.push(wrapper);
  await flushPromises();
  return wrapper;
}

function field(testId: string): HTMLInputElement {
  return document.querySelector(`[data-testid="${testId}"]`) as HTMLInputElement;
}

describe('task editor', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    harness.lists = [
      { id: 'local', account_id: null, provider: 'local', name: '', color: null, read_only: false, gone: false, last_synced_at: null, open_count: 0 },
    ];
    harness.tasks = [task('existing', { title: 'Old title', due_at: '2026-11-10', thread_id: 'th-9', priority: 3 })];
  });

  afterEach(() => {
    while (mounted.length) mounted.pop()!.unmount();
    useTaskEditor().close();
    useToast().state.toasts.splice(0);
  });

  it('turns a thread into a task titled after its subject and linked to it', async () => {
    useTaskEditor().openForThread('th-1', 'Re: Fwd: Quarterly numbers');
    await mountDialog();
    expect(field('task-title').value).toBe('Quarterly numbers');
    expect(document.body.textContent).toContain('Made from a mail.');

    (document.querySelector('[data-testid="task-save"]') as HTMLButtonElement).click();
    await flushPromises();
    expect(tasksBackend.backendCreateTask).toHaveBeenCalledWith(expect.objectContaining({
      listId: 'local',
      threadId: 'th-1',
      fields: expect.objectContaining({ title: 'Quarterly numbers', due_at: null, priority: 0 }),
    }));
    expect(useTaskEditor().state.request).toBeNull();
  });

  it('edits an existing task against the revision it opened and opens its mail', async () => {
    useTaskEditor().openTask('existing');
    await mountDialog();
    expect(field('task-due-date').value).toBe('2026-11-10');
    const title = field('task-title');
    title.value = 'New title';
    title.dispatchEvent(new Event('input'));
    (document.querySelector('[data-testid="task-save"]') as HTMLButtonElement).click();
    await flushPromises();
    expect(tasksBackend.backendUpdateTask).toHaveBeenCalledWith({
      id: 'existing',
      expectedUpdatedAt: '2026-11-01T00:00:00Z',
      // Another client's 3 is "high" here; unchanged, it is sent back as 3.
      fields: expect.objectContaining({ title: 'New title', due_at: '2026-11-10', priority: 3 }),
    });

    useTaskEditor().openTask('existing');
    await flushPromises();
    const openMail = [...document.querySelectorAll('button')].find((button) => button.textContent?.includes('Open mail'));
    openMail!.click();
    expect(harness.push).toHaveBeenCalledWith({ name: 'thread', params: { id: 'th-9' } });
  });

  it('keeps the typed edit after a sync changed the task, and saves it on retry', async () => {
    useTaskEditor().openTask('existing');
    await mountDialog();
    vi.mocked(tasksBackend.backendUpdateTask).mockRejectedValueOnce(new Error('backend.errors.taskChanged'));
    harness.tasks = [task('existing', { title: 'Server title', updated_at: '2026-11-02T00:00:00Z', thread_id: 'th-9' })];
    const title = field('task-title');
    title.value = 'Mine';
    title.dispatchEvent(new Event('input'));
    (document.querySelector('[data-testid="task-save"]') as HTMLButtonElement).click();
    await flushPromises();
    expect(document.body.textContent).toContain('Save again to keep your version.');
    expect(field('task-title').value).toBe('Mine');
    (document.querySelector('[data-testid="task-save"]') as HTMLButtonElement).click();
    await flushPromises();
    expect(tasksBackend.backendUpdateTask).toHaveBeenLastCalledWith(expect.objectContaining({
      expectedUpdatedAt: '2026-11-02T00:00:00Z',
      fields: expect.objectContaining({ title: 'Mine' }),
    }));
  });

  it('refuses an empty title without calling the backend', async () => {
    useTaskEditor().openNew();
    await mountDialog();
    (document.querySelector('[data-testid="task-save"]') as HTMLButtonElement).click();
    await flushPromises();
    expect(tasksBackend.backendCreateTask).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('A task needs a title.');
  });
});
