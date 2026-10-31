// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { effectScope, watch } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { backendTransitionThreads, backendSupportsNativeCommands } from '@/services/backend';
import { useUndoFab } from '@/composables/useUndoFab';
import type { Thread } from '@/types/mail';
import { useThreadsStore } from '@/stores/threads';
import { useToast } from '@/composables/useToast';
import { useWorkflow } from '@/composables/useWorkflow';
import { useThreadActions } from '@/composables/useThreadActions';

vi.mock('vue-router', () => ({ useRoute: () => undefined, useRouter: () => undefined }));

/**
 * Round trips, counted at the only seam that reaches the backend.
 *
 * Every helper here maps to exactly one `#[tauri::command]`, which
 * `backend.ipc-coverage.test.ts` keeps true.
 */
const { calls, rejectFirst } = vi.hoisted(() => ({
  calls: [] as string[],
  rejectFirst: { enabled: false },
}));

vi.mock('@/services/backend', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/backend')>();
  const record = <T>(command: string, result: (input: unknown) => T) =>
    vi.fn(async (input: unknown) => {
      calls.push(command);
      return result(input);
    });
  return {
    ...actual,
    backendCommandsAvailable: () => true,
    backendSupportsNativeCommands: vi.fn(() => true),
    backendTransitionThreads: record('transition_threads', (input) => {
      const threads = (input as { threads: Array<{ thread_id: string }> }).threads;
      if (!rejectFirst.enabled) {
        return { applied: threads.map((entry) => entry.thread_id), failed: [] };
      }
      const [rejected, ...rest] = threads;
      return {
        applied: rest.map((entry) => entry.thread_id),
        failed: [{ thread_id: rejected!.thread_id, error: 'database is locked' }],
      };
    }),
    backendSetThreadsRead: vi.fn(async () => {
      calls.push('set_threads_read');
    }),
    backendDismissMobileNotifications: record('dismiss_mobile_notifications', (threadIds) => {
      dismissedIds.push(threadIds as string[]);
    }),
    backendListThreads: record('list_threads', () => ({
      threads: [],
      next_cursor: null,
      has_more: false,
    })),
    backendListCachedThreads: vi.fn(async () => null),
    backendListMailboxes: record('list_mailboxes', () => []),
    backendListSmartInboxSummary: record('list_smart_inbox_summary', () => null),
    backendRecordFrontendDiagnostic: vi.fn(async () => {}),
  };
});

const dismissedIds: string[][] = [];
const BATCH = 200;

function seedThreads(count: number): Thread[] {
  return Array.from({ length: count }, (_, index) => ({
    id: `t-${index}`,
    accountId: 'acc-1',
    subject: `Betreff ${index}`,
    participants: [{ name: 'Sender', email: 'sender@example.com' }],
    snippet: 'Vorschau',
    lastMessageAt: '2026-08-24T15:08:00.000Z',
    lastMessageAtMs: Date.parse('2026-08-24T15:08:00.000Z'),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox' as const,
    priority: 0,
    isStarred: false,
    hasInvite: false,
    mailboxNames: ['INBOX'],
    tags: [],
  }));
}

function countCommand(name: string): number {
  return calls.filter((command) => command === name).length;
}

describe('bulk workflow transitions', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    calls.length = 0;
    dismissedIds.length = 0;
    rejectFirst.enabled = false;
    vi.mocked(backendSupportsNativeCommands).mockReturnValue(true);
    useToast().clear();
  });

  afterEach(() => useToast().clear());

  it('files a selection in two round trips and one receipt', async () => {
    const threads = useThreadsStore();
    threads.threads = seedThreads(BATCH);
    const actions = useThreadActions();

    await actions.archive(threads.threads.map((thread) => thread.id));

    // Was 3 × 200: one transition, one read flag and one dismissal per thread.
    // The transition now marks Done threads read itself.
    expect(countCommand('transition_threads')).toBe(1);
    expect(countCommand('set_threads_read')).toBe(0);
    expect(countCommand('dismiss_mobile_notifications')).toBe(1);
    expect(dismissedIds[0]).toHaveLength(BATCH);

    const toasts = useToast().state.toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0]).toMatchObject({ groupKey: 'workflow:done', count: BATCH });
  }, 30_000);

  /**
   * The numbers this issue is about. The per-thread path this once measured
   * against is gone: it cost three round trips and one list projection per
   * thread, and nothing in the UI reached it any more.
   */
  it('costs two round trips and one projection for a whole selection', async () => {
    const threads = useThreadsStore();
    threads.threads = seedThreads(BATCH);
    const workflow = useWorkflow();

    const scope = effectScope();
    let projections = 0;
    scope.run(() => watch(() => threads.visibleThreads, () => { projections += 1; }));

    await workflow.transitionMany(
      threads.threads.map((thread) => thread.id),
      'done',
      'manual',
      undefined,
      { refresh: false },
    );
    scope.stop();

    expect(calls.length).toBe(2);
    // The batch changes every row before it publishes once.
    expect(projections).toBeLessThanOrEqual(2);
  }, 30_000);

  it('publishes the optimistic batch as a single store mutation', async () => {
    const threads = useThreadsStore();
    threads.threads = seedThreads(BATCH);
    const workflow = useWorkflow();
    let publications = 0;
    const scope = effectScope();
    // What the virtualized list observes: the array, not the rows.
    scope.run(() => watch(() => threads.threads, () => { publications += 1; }, { flush: 'sync' }));

    const applied = await workflow.transitionMany(
      threads.threads.map((thread) => thread.id),
      'done',
      'manual',
      undefined,
      { refresh: false },
    );

    expect(applied).toHaveLength(BATCH);
    // Was 3 × 200 row writes, spread over as many awaited flushes.
    expect(publications).toBeLessThanOrEqual(2);
    expect(threads.threads.every((thread) => thread.workflowState === 'done')).toBe(true);
    expect(threads.threads.every((thread) => thread.unreadCount === 0)).toBe(true);
    scope.stop();
  }, 30_000);

  it('undoes the whole batch in one round trip per previous state', async () => {
    const threads = useThreadsStore();
    threads.threads = seedThreads(BATCH);
    const actions = useThreadActions();
    await actions.archive(threads.threads.map((thread) => thread.id));
    calls.length = 0;

    const undo = useToast().state.toasts[0]?.action;
    await undo?.batch?.handler(undo.batch.items);

    expect(countCommand('transition_threads')).toBe(1);
    expect(threads.threads.every((thread) => thread.workflowState === 'inbox')).toBe(true);
  }, 30_000);

  it('accepts rapid filing of different threads from one action instance', async () => {
    const threads = useThreadsStore();
    threads.threads = seedThreads(3);
    const actions = useThreadActions();
    await Promise.all([actions.archive(['t-0']), actions.archive(['t-1']), actions.archive(['t-2'])]);
    expect(countCommand('transition_threads')).toBe(3);
    expect(dismissedIds.flat().sort()).toEqual(['t-0', 't-1', 't-2']);
  });

  /** A notification action is acknowledged on this answer, so it must be exact. */
  it('resolves true once every thread is Done, including one that already was', async () => {
    const threads = useThreadsStore();
    threads.threads = [...seedThreads(1), { ...seedThreads(2)[1]!, workflowState: 'done' as const }];

    await expect(useThreadActions().archive(['t-0', 't-1'])).resolves.toBe(true);
    await expect(useThreadActions().archive(['t-0'])).resolves.toBe(true);
    expect(threads.threads.every((thread) => thread.workflowState === 'done')).toBe(true);
  });

  it('resolves false for a thread it could not find or the backend no longer has', async () => {
    const threads = useThreadsStore();
    threads.threads = seedThreads(1);
    await expect(useThreadActions().archive(['missing'])).resolves.toBe(false);
    vi.mocked(backendTransitionThreads).mockResolvedValueOnce({ applied: [], failed: [{ thread_id: 't-0', error: 'thread not found' }] });
    await expect(useThreadActions().archive(['t-0'])).resolves.toBe(false);
  });

  it('undoes by snapshot id after a list load evicts the archived rows', async () => {
    const threads = useThreadsStore();
    threads.threads = seedThreads(2);
    await useThreadActions().archive(['t-0', 't-1']);
    const undo = useToast().state.toasts[0]!.action!;
    threads.threads = [];
    calls.length = 0;
    await undo.batch!.handler(undo.batch!.items);
    expect(countCommand('transition_threads')).toBe(1);
  });

  it('rolls back a failed mock move and offers no success receipt', async () => {
    const threads = useThreadsStore();
    threads.threads = seedThreads(1);
    vi.mocked(backendSupportsNativeCommands).mockReturnValueOnce(false);
    vi.mocked(backendTransitionThreads).mockRejectedValueOnce(new Error('mock move failed'));
    await expect(useThreadActions().archive(['t-0'])).resolves.toBe(false);
    expect(threads.find('t-0')?.workflowState).toBe('inbox');
    expect(useToast().state.toasts.every(item => item.tone === 'error')).toBe(true);
    expect(useToast().state.toasts[0]?.message).toContain('mock move failed');
  });

  it('reports an undo that restores nothing and retains its receipt', async () => {
    const threads = useThreadsStore();
    threads.threads = seedThreads(1);
    await useThreadActions().archive(['t-0']);
    threads.threads = [];
    vi.mocked(backendTransitionThreads).mockResolvedValueOnce({ applied: [], failed: [{ thread_id: 't-0', error: 'thread not found' }] });
    const fab = useUndoFab();
    await fab.run(fab.entries.value[0]!);
    expect(fab.entries.value).toHaveLength(1);
    expect(useToast().state.toasts.some(item => item.tone === 'error' && item.message === 'No conversations could be restored.')).toBe(true);
  });

  it('offers undo only for the threads the backend accepted', async () => {
    rejectFirst.enabled = true;
    const threads = useThreadsStore();
    threads.threads = seedThreads(3);
    const actions = useThreadActions();

    await expect(actions.archive(threads.threads.map((thread) => thread.id))).resolves.toBe(false);

    const toasts = useToast().state.toasts;
    expect(toasts.find((entry) => entry.tone === 'success')).toMatchObject({ count: 2 });
    expect(toasts.find((entry) => entry.tone === 'error')?.message).toContain('database is locked');
    // The rejected row keeps the state it really has, and its unread count.
    expect(threads.threads[0]).toMatchObject({ workflowState: 'inbox', unreadCount: 1 });
    expect(dismissedIds[0]).toEqual(['t-1', 't-2']);
  });
});
