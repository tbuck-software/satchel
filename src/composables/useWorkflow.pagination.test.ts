import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendListThreads,
  backendSetThreadsRead,
  backendTransitionThreads,
} from '@/services/backend';
import { useThreadsStore } from '@/stores/threads';
import { useToast } from './useToast';
import {
  THREAD_LEAVE_MS,
  WORKFLOW_PERSISTENCE_CONCURRENCY,
  useWorkflow,
} from './useWorkflow';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendListCachedThreads: vi.fn(async () => null),
  backendListThreads: vi.fn(),
  backendListMailboxes: vi.fn(async () => []),
  backendListSmartInboxSummary: vi.fn(async () => null),
  backendRecordFrontendDiagnostic: vi.fn(async () => {}),
  backendArchiveThreads: vi.fn(async () => {}),
  backendTransitionThreads: vi.fn(),
  backendGetThread: vi.fn(),
  backendMarkThreadsSpam: vi.fn(),
  backendSetThreadCategory: vi.fn(),
  backendSetThreadMuted: vi.fn(),
  backendSetThreadsRead: vi.fn(async () => {}),
  backendSetThreadsStarred: vi.fn(),
  backendTrashThreads: vi.fn(),
  backendUpdateThreadOrganization: vi.fn(),
  backendDismissMobileNotifications: vi.fn(async () => {}),
}));

const listThreads = vi.mocked(backendListThreads);
const transitionThreads = vi.mocked(backendTransitionThreads);
const setThreadsRead = vi.mocked(backendSetThreadsRead);

const TOTAL = 546;
const PAGE = 200;

/** Default backend: every requested thread moves. */
function applyAll(input: { threads: Array<{ thread_id: string }> }) {
  return { applied: input.threads.map((entry) => entry.thread_id), failed: [] };
}

/** Nothing moves; every requested thread fails with the same message. */
function failAll(error: string) {
  return (input: { threads: Array<{ thread_id: string }> }) => ({
    applied: [],
    failed: input.threads.map((entry) => ({ thread_id: entry.thread_id, error })),
  });
}

function makeRow(index: number) {
  const date = new Date(Date.UTC(2026, 0, 1) + (TOTAL - index) * 60_000).toISOString();
  return {
    id: `t${index}`,
    account_id: 'acc',
    subject: `Betreff ${index}`,
    snippet: '',
    participants: '[]',
    last_message_date: date,
    message_count: 1,
    unread_count: 0,
    is_starred: false,
    is_muted: false,
    workflow_state: 'inbox',
    tags: [],
    mailbox_names: '["INBOX"]',
    has_attachments: false,
    priority: 1,
  };
}

const archived = new Set<string>();

/** Cursor-honouring fake backend over a fixed dataset, minus archived ids. */
function fakeList(cursor?: { id: string } | null) {
  const rows = Array.from({ length: TOTAL }, (_, index) => makeRow(index)).filter(
    (row) => !archived.has(row.id),
  );
  const start = cursor ? rows.findIndex((row) => row.id === cursor.id) + 1 : 0;
  const page = rows.slice(start, start + PAGE);
  const hasMore = start + PAGE < rows.length;
  return {
    threads: page,
    next_cursor: hasMore ? { id: page[page.length - 1]!.id } : null,
    has_more: hasMore,
  };
}

function resetBackend() {
  listThreads.mockReset();
  listThreads.mockImplementation(
    async (input) => fakeList((input as { cursor?: { id: string } } | undefined)?.cursor) as never,
  );
  transitionThreads.mockReset();
  transitionThreads.mockImplementation(async (input) => applyAll(input) as never);
  setThreadsRead.mockReset();
  setThreadsRead.mockResolvedValue(undefined);
}

describe('useWorkflow refresh with a fully paged list', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    archived.clear();
    resetBackend();
    useToast().clear();
    vi.useFakeTimers();
  });

  afterEach(() => {
    useToast().clear();
    vi.useRealTimers();
  });

  /**
   * Regression: archiving used to run load(), which replaced the array with
   * page one — 300+ paged-in threads vanished, the frequent-contacts sidebar
   * recomputed on the gutted list, and two catch-up pages made everything
   * reflow twice on every single check-off.
   */
  it('keeps the paged-in tail and the exhausted cursor across an archive', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();

    await threads.load();
    await threads.loadMore();
    await threads.loadMore();
    expect(threads.threads).toHaveLength(TOTAL);
    expect(threads.hasMoreThreads).toBe(false);

    let minLength = threads.threads.length;
    threads.$subscribe(() => {
      minLength = Math.min(minLength, threads.threads.length);
    });

    archived.add('t100');
    const pending = workflow.transitionMany(['t100'], 'done');
    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);
    await pending;
    await vi.runAllTimersAsync();

    // The archived row keeps its local copy (state done); nothing else drops.
    expect(minLength).toBeGreaterThanOrEqual(TOTAL);
    expect(threads.hasMoreThreads).toBe(false);
    // Initial pagination is three fetches; the refresh adds exactly one head
    // merge — no cursor reset, no catch-up chain.
    expect(listThreads.mock.calls.length).toBe(4);
  });

  it('removes a stale projection when the backend says the thread no longer exists', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    await threads.load();
    transitionThreads.mockImplementationOnce(
      async (input) => failAll('thread not found')(input) as never,
    );

    const pending = workflow.transitionMany(['t0'], 'done');
    const result = expect(pending).resolves.toEqual([]);
    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);

    await result;
    expect(threads.threads.some((thread) => thread.id === 't0')).toBe(false);
  });

  it('restores unread state and merges equivalent workflow failures', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    await threads.load();
    threads.threads[0]!.unreadCount = 3;
    threads.threads[1]!.unreadCount = 2;
    transitionThreads.mockImplementation(async (input) => failAll('db locked')(input) as never);

    const pending = workflow.transitionMany(['t0', 't1'], 'done', 'manual', undefined, {
      refresh: false,
    });
    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);

    await expect(pending).resolves.toEqual([]);
    expect(threads.threads[0]!.unreadCount).toBe(3);
    expect(threads.threads[1]!.unreadCount).toBe(2);
    // Equivalent failures merge into one receipt instead of one per thread.
    expect(useToast().state.toasts).toEqual([
      expect.objectContaining({ groupKey: 'workflow-error:done:db locked', count: 2 }),
    ]);
  });

  it('leaves marking read to the backend transition', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    await threads.load();
    threads.threads[0]!.unreadCount = 2;

    const pending = workflow.transitionMany(['t0'], 'done', 'manual', undefined, {
      refresh: false,
    });
    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);

    await expect(pending).resolves.toEqual(['t0']);
    expect(threads.threads[0]!.workflowState).toBe('done');
    expect(threads.threads[0]!.unreadCount).toBe(0);
    // A follow-up read request finds the scheduled move and is refused.
    expect(setThreadsRead).not.toHaveBeenCalled();
    expect(useToast().state.toasts).toEqual([]);
  });

  /**
   * The semaphore now bounds concurrent *batches* — one selection is a single
   * round trip, but rules, undo and a second pane can still overlap.
   */
  it('bounds backend persistence when many batches overlap', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    await threads.load();
    let active = 0;
    let peak = 0;
    const releases: Array<() => void> = [];
    transitionThreads.mockImplementation((input) => new Promise((resolve) => {
      active += 1;
      peak = Math.max(peak, active);
      releases.push(() => {
        active -= 1;
        resolve(applyAll(input) as never);
      });
    }));

    const pending = Promise.all(
      Array.from({ length: 100 }, (_, index) =>
        workflow.transitionMany([`t${index}`], 'done', 'manual', undefined, { refresh: false }),
      ),
    );
    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);
    expect(transitionThreads).toHaveBeenCalledTimes(WORKFLOW_PERSISTENCE_CONCURRENCY);

    while (transitionThreads.mock.calls.length < 100) {
      releases.splice(0).forEach((release) => release());
      await vi.advanceTimersByTimeAsync(0);
    }
    releases.splice(0).forEach((release) => release());
    await pending;

    expect(peak).toBe(WORKFLOW_PERSISTENCE_CONCURRENCY);
    expect(transitionThreads).toHaveBeenCalledTimes(100);
  });
});

/**
 * Regression for "workflow state changed concurrently (expected inbox, found
 * done)": a stale list projection offered threads the backend had already
 * filed. "Alle abhaken" used to roll them back into the stale state and raise
 * one alarming grouped error toast instead of converging.
 */
describe('useWorkflow concurrent-transition convergence', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    resetBackend();
    useToast().clear();
    vi.useFakeTimers();
  });

  afterEach(() => {
    useToast().clear();
    vi.useRealTimers();
  });

  const concurrencyError = (found: string) =>
    `workflow state changed concurrently (expected inbox, found ${found})`;

  it('treats an already-applied target state as success without an error toast', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    await threads.load();
    const stale = threads.threads.slice(0, 4);
    transitionThreads.mockImplementation(
      async (input) => failAll(concurrencyError('done'))(input) as never,
    );

    const pending = workflow.transitionMany(
      stale.map((thread) => thread.id),
      'done',
      'manual',
      undefined,
      { refresh: false },
    );
    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);

    await expect(pending).resolves.toEqual(stale.map((thread) => thread.id));
    expect(stale.map((thread) => threads.find(thread.id)?.workflowState)).toEqual(['done', 'done', 'done', 'done']);
    // The transition that filed them already marked them read.
    expect(setThreadsRead).not.toHaveBeenCalled();
    // The reported bug surfaced exactly here: four grouped failure toasts.
    expect(useToast().state.toasts).toEqual([]);
  });

  it('adopts the authoritative state instead of resurrecting a stale one', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    await threads.load();
    threads.threads[0]!.unreadCount = 2;
    transitionThreads.mockImplementationOnce(
      async (input) => failAll(concurrencyError('aside'))(input) as never,
    );

    const pending = workflow.transitionMany(['t0'], 'done', 'manual', undefined, {
      refresh: false,
    });
    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);

    await expect(pending).resolves.toEqual([]);
    // Adopted the server truth, not the pre-click belief.
    expect(threads.threads[0]!.workflowState).toBe('aside');
    expect(threads.threads[0]!.unreadCount).toBe(2);
    expect(useToast().state.toasts).toEqual([]);
  });

  it('still rolls back and reports unparseable backend failures', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    await threads.load();
    threads.threads[0]!.unreadCount = 1;
    transitionThreads.mockImplementation(async (input) => failAll('db locked')(input) as never);

    const pending = workflow.transitionMany(['t0'], 'done', 'manual', undefined, {
      refresh: false,
    });
    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);

    await expect(pending).resolves.toEqual([]);
    expect(threads.threads[0]!.workflowState).toBe('inbox');
    expect(threads.threads[0]!.unreadCount).toBe(1);
    expect(useToast().state.toasts).toEqual([
      expect.objectContaining({ groupKey: 'workflow-error:done:db locked' }),
    ]);
  });

  it('rolls the whole batch back when the round trip itself fails', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    await threads.load();
    transitionThreads.mockRejectedValue(new Error('db locked'));

    const pending = workflow.transitionMany(['t0', 't1'], 'done', 'manual', undefined, {
      refresh: false,
    });
    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);

    await expect(pending).resolves.toEqual([]);
    expect(threads.threads[0]!.workflowState).toBe('inbox');
    expect(threads.threads[1]!.workflowState).toBe('inbox');
    expect(useToast().state.toasts).toEqual([
      expect.objectContaining({ groupKey: 'workflow-error:done:db locked', count: 2 }),
    ]);
  });
});
