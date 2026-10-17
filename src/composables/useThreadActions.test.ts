import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const trashMock = vi.fn();
const readMock = vi.fn();
const moveMock = vi.fn();
const blockMock = vi.fn();
const starMock = vi.fn((threadIds: string[], starred: boolean) => {
  for (const thread of threadState.threads) {
    if (threadIds.includes(thread.id as string)) thread.isStarred = starred;
  }
});
const transitionMock = vi.fn();
const priorityMock = vi.fn();
const transitionManyMock = vi.fn();
const loadThreadsMock = vi.fn();
const refreshHeadMock = vi.fn();
const loadMailboxesMock = vi.fn();
const setWorkflowStateMock = vi.fn((threadId: string, state: string, snoozeUntil?: string) => {
  const thread = threadState.threads.find((entry) => entry.id === threadId);
  if (!thread) return;
  thread.workflowState = state;
  thread.snoozeUntil = snoozeUntil;
});
const markReadMock = vi.fn((threadId: string, read: boolean) => {
  const thread = threadState.threads.find((entry) => entry.id === threadId);
  if (thread && read) thread.unreadCount = 0;
});
const applyWorkflowStatesMock = vi.fn((mutations: Array<Record<string, unknown>>) => {
  for (const mutation of mutations) {
    const thread = threadState.threads.find((entry) => entry.id === mutation.threadId);
    if (!thread) continue;
    thread.workflowState = mutation.state;
    thread.snoozeUntil = mutation.snoozeUntil;
    if (mutation.unreadCount !== undefined) thread.unreadCount = mutation.unreadCount;
  }
});

const threadState = {
  threads: [] as Array<Record<string, unknown>>,
  find: (threadId: string) => threadState.threads.find((entry) => entry.id === threadId),
  ensureThreadLoaded: vi.fn(),
  clearSelection: vi.fn(),
  focusNeighborThread: vi.fn(),
  setWorkflowState: setWorkflowStateMock,
  persistWorkflowState: vi.fn(async (id: string, state: string, wake?: string) => setWorkflowStateMock(id, state, wake)),
  applyWorkflowStates: applyWorkflowStatesMock,
  markRead: markReadMock,
  load: loadThreadsMock,
  // Post-action refreshes merge (refreshHead) instead of reloading page one.
  refreshHead: refreshHeadMock,
  // Owns the optimistic removal and its rollback.
  trashThreads: (...args: unknown[]) => trashMock(...args),
  setThreadsRead: (...args: unknown[]) => readMock(...args),
  // Owns the optimistic star and its rollback.
  setThreadsStarred: (...args: [string[], boolean]) => starMock(...args),
  setPriority: (...args: unknown[]) => priorityMock(...args),
};

const mailboxState = {
  mailboxes: [] as Array<Record<string, unknown>>,
  load: loadMailboxesMock,
};

const toasts: Array<{
  message: string;
  tone: string;
  count?: number;
  groupKey?: string;
  action?: {
    label: string;
    handler: () => unknown | Promise<unknown>;
    batch?: {
      key: string;
      items: unknown[];
      handler: (items: unknown[]) => unknown | Promise<unknown>;
    };
  };
}> = [];

vi.mock('@/services/backend', () => ({
  backendBlockSender: (...args: unknown[]) => blockMock(...args),
  backendMoveThreadsToFolder: (...args: unknown[]) => moveMock(...args),
  backendCommandsAvailable: () => true,
  backendSupportsNativeCommands: () => true,
}));

// useThreadActions pulls in the topics store for addToTopic/removeFromTopic;
// stub it out so the backend mock above stays minimal.
const topicAddMock = vi.hoisted(() => vi.fn());
const topicRemoveMock = vi.hoisted(() => vi.fn());
vi.mock('@/stores/topics', () => ({
  useTopicsStore: () => ({
    byId: () => undefined,
    addItemsToTopic: topicAddMock,
    removeItemFromTopic: topicRemoveMock,
  }),
}));

vi.mock('@/stores/threads', () => ({ useThreadsStore: () => threadState }));
vi.mock('@/stores/mailboxes', () => ({ useMailboxesStore: () => mailboxState }));
vi.mock('@/composables/useWorkflow', () => ({
  useWorkflow: () => ({
    transition: (...args: unknown[]) => transitionMock(...args),
    transitionMany: (...args: unknown[]) => transitionManyMock(...args),
  }),
}));
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({
    success: (message: string, opts?: Record<string, unknown>) => {
      toasts.push({
        message,
        tone: 'success',
        count: opts?.count as number | undefined,
        groupKey: opts?.groupKey as string | undefined,
        action: opts?.action as never,
      });
    },
    info: (message: string, opts?: Record<string, unknown>) => {
      toasts.push({ message, tone: 'info', action: opts?.action as never });
    },
    error: (message: string) => {
      toasts.push({ message, tone: 'error' });
    },
  }),
}));

import { useThreadActions } from './useThreadActions';

function seedThread(overrides: Record<string, unknown> = {}) {
  return {
    id: 'thread-1',
    accountId: 'acc-1',
    workflowState: 'inbox',
    isStarred: false,
    unreadCount: 0,
    ...overrides,
  };
}

async function runGroupedUndo() {
  const actions = toasts.flatMap((entry) => entry.action ? [entry.action] : []);
  const batch = actions[0]?.batch;
  if (batch && actions.every((action) => action.batch?.key === batch.key)) {
    await batch.handler(actions.flatMap((action) => action.batch?.items ?? []));
    return;
  }
  for (const action of actions) await action.handler();
}

describe('useThreadActions', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    toasts.length = 0;
    for (const mock of [trashMock, moveMock, blockMock, transitionMock, transitionManyMock, loadThreadsMock, refreshHeadMock, loadMailboxesMock]) {
      mock.mockReset();
      mock.mockResolvedValue(undefined);
    }
    // `transition` reports whether the backend accepted the change;
    // `transitionMany` reports the ids it actually changed.
    transitionMock.mockResolvedValue(true);
    transitionManyMock.mockImplementation(async (threadIds: string[]) => [...threadIds]);
    threadState.ensureThreadLoaded.mockReset();
    threadState.persistWorkflowState.mockClear();
    threadState.clearSelection.mockReset();
    threadState.focusNeighborThread.mockReset();
    // Keeps its store-like implementation; mockReset would drop it.
    starMock.mockClear();
    setWorkflowStateMock.mockClear();
    applyWorkflowStatesMock.mockClear();
    markReadMock.mockClear();
    threadState.threads = [seedThread()];
    mailboxState.mailboxes = [{ id: 'folder-inbox', account_id: 'acc-1', mailbox_type: 'inbox' }];
  });

  it('serializes one thread across action instances and snapshots its latest state', async () => {
    let finish!: (ids: string[]) => void;
    transitionManyMock.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const first = useThreadActions().archive(['thread-1']);
    const second = useThreadActions().setAside(['thread-1']);
    expect(transitionManyMock).toHaveBeenCalledTimes(1);
    finish(['thread-1']);
    await Promise.all([first, second]);
    expect(transitionManyMock).toHaveBeenCalledTimes(2);
    expect(toasts.at(-1)?.action?.batch?.items).toEqual([
      { threadId: 'thread-1', previousState: 'done', expectedState: 'aside' },
    ]);
  });

  it.each(['setAside', 'trash'] as const)('rehydrates a queued %s after the preceding action evicts its row', async (action) => {
    let finish!: (ids: string[]) => void;
    transitionManyMock.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    refreshHeadMock.mockImplementationOnce(async () => { threadState.threads = []; });
    threadState.ensureThreadLoaded.mockImplementationOnce(async () => {
      threadState.threads = [seedThread({ workflowState: 'done' })];
    });
    const actions = useThreadActions();
    const first = actions.archive(['thread-1']);
    const second = actions[action](['thread-1']);
    finish(['thread-1']);
    await Promise.all([first, second]);
    expect(threadState.ensureThreadLoaded).toHaveBeenCalledExactlyOnceWith('thread-1');
    if (action === 'trash') expect(trashMock).toHaveBeenCalledExactlyOnceWith(['thread-1']);
    else expect(transitionManyMock).toHaveBeenLastCalledWith(['thread-1'], 'aside', 'manual', undefined, { refresh: false });
    expect(toasts.at(-1)?.action).toBeDefined();
  });

  it('offers undo after archiving and restores the previous state', async () => {
    threadState.threads = [seedThread({ workflowState: 'inbox' })];
    const actions = useThreadActions();

    await actions.archive(['thread-1']);

    // The batch defers the (expensive) list refresh to a single call at the end.
    expect(transitionManyMock).toHaveBeenCalledWith(['thread-1'], 'done', 'manual', undefined, { refresh: false });
    expect(refreshHeadMock).toHaveBeenCalledTimes(1);
    const toast = toasts.at(-1);
    expect(toast?.action?.label).toBe('Undo');

    transitionManyMock.mockClear();
    toast!.action!.handler();
    await vi.waitFor(() =>
      expect(transitionManyMock).toHaveBeenCalledWith(['thread-1'], 'inbox', 'manual', undefined, expect.objectContaining({ refresh: false })),
    );
  });

  it('starts the visible undo window before a slow list refresh finishes', async () => {
    let finishRefresh!: () => void;
    refreshHeadMock.mockImplementationOnce(
      () => new Promise<void>((resolve) => { finishRefresh = resolve; }),
    );
    const actions = useThreadActions();

    const pending = actions.archive(['thread-1']);
    await vi.waitFor(() => expect(toasts.at(-1)?.action?.label).toBe('Undo'));
    expect(refreshHeadMock).toHaveBeenCalledTimes(1);

    finishRefresh();
    await pending;
  });

  it('does not let a stale Inbox projection resurface completed smart-folder mail', async () => {
    threadState.threads = [seedThread({ unreadCount: 2 })];
    transitionManyMock.mockImplementationOnce(async (threadIds: string[]) => {
      // Closing the emptied folder can publish a list page that was read just
      // before the durable transition committed.
      threadState.threads[0] = seedThread({ unreadCount: 2, workflowState: 'inbox' });
      return [...threadIds];
    });
    const actions = useThreadActions();

    await actions.archive(['thread-1']);

    expect(threadState.threads[0]).toMatchObject({
      id: 'thread-1',
      workflowState: 'done',
      unreadCount: 0,
    });
    expect(applyWorkflowStatesMock).toHaveBeenCalledTimes(1);
  });

  it('files a large bulk archive as one batch, one receipt and one refresh', async () => {
    threadState.threads = Array.from({ length: 300 }, (_, index) =>
      seedThread({ id: `thread-${index}` }),
    );
    const actions = useThreadActions();

    await actions.archive(threadState.threads.map((thread) => thread.id as string));

    // Was one transition, one toast and one store write per thread.
    expect(transitionManyMock).toHaveBeenCalledOnce();
    expect(toasts).toHaveLength(1);
    expect(toasts[0]).toMatchObject({ groupKey: 'workflow:done', count: 300 });
    expect(applyWorkflowStatesMock).toHaveBeenCalledOnce();
    expect(refreshHeadMock).toHaveBeenCalledOnce();
  });

  it('offers undo only for the threads the backend actually changed', async () => {
    threadState.threads = [seedThread({ id: 'a' }), seedThread({ id: 'b' })];
    transitionManyMock.mockResolvedValueOnce(['a']);
    const actions = useThreadActions();

    await actions.archive(['a', 'b']);

    expect(toasts).toHaveLength(1);
    expect(toasts[0]).toMatchObject({ count: 1 });
    expect(toasts[0]?.action?.batch?.items).toEqual([{ threadId: 'a', previousState: 'inbox', expectedState: 'done' }]);
  });

  it('stays quiet when the backend rejected every thread', async () => {
    transitionManyMock.mockResolvedValue([]);
    const actions = useThreadActions();

    await actions.archive(['thread-1']);

    // The failure reports itself; a success toast with a dead undo would not.
    expect(toasts).toEqual([]);
  });

  it('restores the state each thread actually had, not a fixed default', async () => {
    threadState.threads = [
      seedThread({ id: 'a', workflowState: 'inbox' }),
      seedThread({ id: 'b', workflowState: 'snoozed' }),
    ];
    const actions = useThreadActions();

    await actions.archive(['a', 'b']);
    transitionManyMock.mockClear();
    await runGroupedUndo();

    // One round trip per distinct previous state, not one per thread.
    await vi.waitFor(() => {
      expect(transitionManyMock).toHaveBeenCalledWith(['a'], 'inbox', 'manual', undefined, expect.objectContaining({ refresh: false }));
      expect(transitionManyMock).toHaveBeenCalledWith(['b'], 'snoozed', 'manual', undefined, expect.objectContaining({ refresh: false }));
    });
  });

  it('restores each snoozed thread to its original wake time', async () => {
    const early = '2026-10-01T09:00:00Z';
    const late = '2026-10-08T09:00:00Z';
    threadState.threads = [
      seedThread({ id: 'a', workflowState: 'snoozed', snoozeUntil: early }),
      seedThread({ id: 'b', workflowState: 'snoozed', snoozeUntil: late }),
      seedThread({ id: 'c', workflowState: 'snoozed', snoozeUntil: early }),
    ];
    const actions = useThreadActions();
    await actions.archive(['a', 'b', 'c']);
    transitionManyMock.mockClear();
    await runGroupedUndo();
    expect(transitionManyMock).toHaveBeenCalledWith(['a', 'c'], 'snoozed', 'manual', early, expect.objectContaining({ refresh: false }));
    expect(transitionManyMock).toHaveBeenCalledWith(['b'], 'snoozed', 'manual', late, expect.objectContaining({ refresh: false }));
  });

  it('sets a thread aside without a wake time and offers undo', async () => {
    const actions = useThreadActions();

    await actions.setAside(['thread-1']);

    expect(transitionManyMock).toHaveBeenCalledWith(
      ['thread-1'],
      'aside',
      'manual',
      undefined,
      { refresh: false },
    );
    expect(toasts.at(-1)?.message).toContain('set aside');
    expect(toasts.at(-1)?.action?.label).toBe('Undo');
  });

  it('keeps waiting-for-reply distinct from a dated reminder', async () => {
    const actions = useThreadActions();

    await actions.waitForReply(['thread-1']);

    expect(transitionManyMock).toHaveBeenCalledWith(
      ['thread-1'],
      'waiting',
      'manual',
      undefined,
      { refresh: false },
    );
    expect(toasts.at(-1)?.message).toContain('waiting for a reply');
  });

  it('offers undo after trashing and moves threads back to the inbox folder', async () => {
    const actions = useThreadActions();

    await actions.trash(['thread-1']);

    expect(threadState.focusNeighborThread).toHaveBeenCalledWith('thread-1');
    expect(trashMock).toHaveBeenCalledWith(['thread-1']);
    const toast = toasts.at(-1);
    expect(toast?.action?.label).toBe('Undo');

    toast!.action!.handler();
    await vi.waitFor(() =>
      expect(moveMock).toHaveBeenCalledWith({ threadIds: ['thread-1'], targetFolderId: 'folder-inbox' }),
    );
  });

  it('restores the wake time after undoing trash', async () => {
    const wake = '2026-10-10T07:30:00Z';
    threadState.threads = [seedThread({ workflowState: 'snoozed', snoozeUntil: wake })];
    const actions = useThreadActions();
    await actions.trash(['thread-1']);
    toasts.at(-1)!.action!.handler();
    await vi.waitFor(() => expect(threadState.persistWorkflowState).toHaveBeenCalledWith(
      'thread-1', 'snoozed', wake,
    ));
  });

  it('moves folders through the remote-first action and refreshes both projections', async () => {
    const actions = useThreadActions();

    await actions.moveToFolder(['thread-1', 'thread-1'], 'folder-work', 'Work');

    expect(moveMock).toHaveBeenCalledWith({
      threadIds: ['thread-1'],
      targetFolderId: 'folder-work',
    });
    expect(moveMock.mock.invocationCallOrder[0]).toBeLessThan(loadThreadsMock.mock.invocationCallOrder[0]!);
    expect(loadThreadsMock).toHaveBeenCalledOnce();
    expect(loadMailboxesMock).toHaveBeenCalledOnce();
    expect(toasts.at(-1)).toMatchObject({ tone: 'success', message: 'Moved to “Work”.' });
  });

  it('blocks a sender before focusing and refreshing its originating threads', async () => {
    const events: string[] = [];
    blockMock.mockImplementationOnce(async (input: unknown) => {
      events.push('block');
      expect(input).toEqual({ pattern: 'sender@example.com', accountId: 'acc-1' });
    });
    threadState.focusNeighborThread.mockImplementation((threadId: string) => {
      events.push(`focus:${threadId}`);
    });
    const actions = useThreadActions();

    await actions.blockSender({
      pattern: 'sender@example.com',
      accountId: 'acc-1',
      threadIds: ['thread-1', 'thread-1'],
    });

    expect(events).toEqual(['block', 'focus:thread-1']);
    expect(refreshHeadMock).toHaveBeenCalledOnce();
    expect(loadMailboxesMock).toHaveBeenCalledOnce();
  });

  it('propagates a failed sender block without changing local thread state', async () => {
    blockMock.mockRejectedValueOnce(new Error('offline'));
    const actions = useThreadActions();

    await expect(actions.blockSender({
      pattern: 'sender@example.com',
      accountId: 'acc-1',
      threadIds: ['thread-1'],
    })).rejects.toThrow('offline');

    expect(threadState.focusNeighborThread).not.toHaveBeenCalled();
  });

  it('does not offer a dead undo when no inbox folder is known', async () => {
    mailboxState.mailboxes = [];
    const actions = useThreadActions();

    await actions.trash(['thread-1']);

    const toast = toasts.at(-1);
    expect(toast?.message).toContain('deleted');
    expect(toast?.action).toBeUndefined();
  });

  it('surfaces a failed trash instead of claiming success', async () => {
    trashMock.mockRejectedValueOnce(new Error('offline'));
    const actions = useThreadActions();

    await actions.trash(['thread-1']);

    expect(toasts.at(-1)).toMatchObject({ tone: 'error', message: 'offline' });
  });

  it('delegates pinning to the store instead of calling the backend itself', async () => {
    const actions = useThreadActions();

    await actions.togglePinned('thread-1');

    expect(starMock).toHaveBeenCalledTimes(1);
    expect(starMock).toHaveBeenCalledWith(['thread-1'], true);
    expect(threadState.threads[0]!.isStarred).toBe(true);
    expect(refreshHeadMock).toHaveBeenCalled();
  });

  it('skips threads that already carry the requested star', async () => {
    threadState.threads[0]!.isStarred = true;
    const actions = useThreadActions();

    await actions.setPinned(['thread-1'], true);

    expect(starMock).not.toHaveBeenCalled();
  });

  it('reports a refused star and leaves the store to roll it back', async () => {
    starMock.mockRejectedValueOnce(new Error('nope'));
    const actions = useThreadActions();

    await actions.togglePinned('thread-1');

    expect(threadState.threads[0]!.isStarred).toBe(false);
    expect(toasts.at(-1)?.tone).toBe('error');
    expect(refreshHeadMock).not.toHaveBeenCalled();
  });

  it('ignores actions on an empty selection', async () => {
    const actions = useThreadActions();

    await actions.archive([]);
    await actions.trash([]);

    expect(transitionManyMock).not.toHaveBeenCalled();
    expect(trashMock).not.toHaveBeenCalled();
    expect(toasts).toHaveLength(0);
  });
});


describe('mail action failure feedback', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    readMock.mockReset();
  });
  it('reports a failed read action without a success result or toast', async () => {
    readMock.mockRejectedValueOnce(new Error('database is locked'));
    toasts.length = 0;
    const actions = useThreadActions();
    await expect(actions.setRead(['thread-1'], true)).resolves.toBe(false);
    expect(toasts).toEqual([{ message: 'database is locked', tone: 'error' }]);
  });

  it('returns success only after persistence completes', async () => {
    readMock.mockResolvedValueOnce(undefined);
    toasts.length = 0;
    const actions = useThreadActions();
    await expect(actions.setRead(['thread-1'], true)).resolves.toBe(true);
    expect(toasts).toHaveLength(0);
  });
});

describe('silent failures (#119)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    toasts.length = 0;
    topicAddMock.mockReset();
    topicRemoveMock.mockReset();
    priorityMock.mockReset();
    threadState.threads = [seedThread({ priority: 1 })];
  });

  it('reports a failed topic undo and keeps the receipt', async () => {
    topicAddMock.mockResolvedValue(undefined);
    topicRemoveMock.mockRejectedValue(new Error('topic store offline'));
    const actions = useThreadActions();
    await actions.addToTopic(['thread-1'], 'topic-1');

    const undo = toasts[0]?.action;
    await expect(undo?.handler()).resolves.toBe(false);
    expect(toasts.at(-1)).toEqual({ message: 'topic store offline', tone: 'error' });
  });

  it('reports a failed re-add when undoing a topic removal', async () => {
    topicRemoveMock.mockResolvedValue(undefined);
    topicAddMock.mockRejectedValue(new Error('re-add refused'));
    const actions = useThreadActions();
    await actions.removeFromTopic(['thread-1'], 'topic-1');

    await expect(toasts[0]?.action?.handler()).resolves.toBe(false);
    expect(toasts.at(-1)).toEqual({ message: 're-add refused', tone: 'error' });
  });

  it('toggles priority through the store and reports a refusal', async () => {
    priorityMock.mockRejectedValueOnce(new Error('priority refused'));
    const actions = useThreadActions();

    await actions.togglePriority('thread-1');

    expect(priorityMock).toHaveBeenCalledWith('thread-1', 2);
    expect(toasts).toEqual([{ message: 'priority refused', tone: 'error' }]);
  });
});
