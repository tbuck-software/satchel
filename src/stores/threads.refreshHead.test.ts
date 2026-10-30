import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendGetThread,
  backendGetThreads,
  backendListCachedThreads,
  backendListSmartInboxSummary,
  backendListThreads,
} from '@/services/backend';
import { useAccountsStore } from './accounts';
import { useThreadsStore } from './threads';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendListCachedThreads: vi.fn(),
  backendListThreads: vi.fn(),
  backendListSmartInboxSummary: vi.fn(),
  backendGetThread: vi.fn(),
  backendGetThreads: vi.fn(),
  backendMarkThreadsSpam: vi.fn(),
  backendSetThreadCategory: vi.fn(),
  backendSetThreadMuted: vi.fn(),
  backendSetThreadsRead: vi.fn(),
  backendSetThreadsStarred: vi.fn(),
  backendTrashThreads: vi.fn(),
  backendUpdateThreadOrganization: vi.fn(),
}));

const listThreads = vi.mocked(backendListThreads);
const listCachedThreads = vi.mocked(backendListCachedThreads);
const listSummary = vi.mocked(backendListSmartInboxSummary);
const getThread = vi.mocked(backendGetThread);

function row(id: string, date: string) {
  return {
    id,
    account_id: 'acc',
    subject: `Betreff ${id}`,
    snippet: '',
    participants: [],
    last_message_date: date,
    message_count: 1,
    unread_count: 0,
    is_starred: false,
    is_muted: false,
    workflow_state: 'todo',
    tags: [],
    mailbox_names: JSON.stringify(['INBOX']),
    folder_type: 'inbox',
  };
}

function page(rows: ReturnType<typeof row>[], cursor: unknown, hasMore: boolean) {
  return { threads: rows, next_cursor: cursor, has_more: hasMore };
}

describe('threads.refreshHead', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    listThreads.mockReset();
    listCachedThreads.mockReset();
    listCachedThreads.mockResolvedValue(null);
    listSummary.mockReset();
    getThread.mockReset();
    vi.mocked(backendGetThreads).mockReset().mockImplementation(async ids => {
      const pages = await Promise.all(listThreads.mock.results.filter(result => result.type === 'return').map(result => result.value.catch(() => null)));
      const known = new Map(pages.flatMap(page => page?.threads ?? []).map(row => [row.id, row]));
      return ids.flatMap(id => known.has(id) ? [known.get(id)!] : []);
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('shows the cached first page while the current projection revalidates', async () => {
    const threads = useThreadsStore();
    let resolveFresh!: (value: ReturnType<typeof page>) => void;
    listCachedThreads.mockResolvedValueOnce(
      page([row('cached', '2026-08-09T00:00:00Z')], { id: 'cached' }, true) as never,
    );
    listThreads.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveFresh = resolve;
      }) as never,
    );

    const loading = threads.load();
    await vi.waitFor(() => expect(threads.threads.map((thread) => thread.id)).toEqual(['cached']));

    resolveFresh(page([row('fresh', '2026-08-10T00:00:00Z')], null, false));
    await loading;
    expect(threads.threads.map((thread) => thread.id)).toEqual(['fresh']);
  });

  it('prefetches exactly one more page when the browser becomes idle', async () => {
    const threads = useThreadsStore();
    const idleCallbacks: Array<() => void> = [];
    vi.stubGlobal('window', {
      requestIdleCallback: vi.fn((callback: () => void) => {
        idleCallbacks.push(callback);
        return 7;
      }),
      cancelIdleCallback: vi.fn(),
      clearTimeout: vi.fn(),
      setTimeout: vi.fn(),
    });
    listThreads
      .mockResolvedValueOnce(
        page([row('t2', '2026-08-10T00:00:00Z')], {
          id: 't2',
          last_message_date: '2026-08-10T00:00:00Z',
        }, true) as never,
      )
      .mockResolvedValueOnce(
        page([row('t1', '2026-08-09T00:00:00Z')], null, false) as never,
      );

    await threads.load();
    expect(listThreads).toHaveBeenCalledTimes(1);
    expect(idleCallbacks).toHaveLength(1);

    idleCallbacks[0]!();
    await vi.waitFor(() => expect(threads.threads.map((thread) => thread.id)).toEqual(['t2', 't1']));
    expect(listThreads).toHaveBeenCalledTimes(2);
  });

  it('keeps threads paged in past the first page when new mail arrives', async () => {
    const threads = useThreadsStore();

    // Page 1, then a second page appended through loadMore().
    listThreads.mockResolvedValueOnce(
      page([row('t3', '2026-07-31T10:00:00Z'), row('t2', '2026-07-30T10:00:00Z')], { id: 't2' }, true) as never,
    );
    await threads.load();
    listThreads.mockResolvedValueOnce(
      page([row('t1', '2026-07-29T10:00:00Z')], { id: 't1' }, true) as never,
    );
    await threads.loadMore();
    expect(threads.threads.map((t) => t.id)).toEqual(['t3', 't2', 't1']);

    // New mail: the head page now leads with t4.
    listThreads.mockResolvedValueOnce(
      page([row('t4', '2026-08-01T10:00:00Z'), row('t3', '2026-07-31T10:00:00Z')], { id: 't3' }, true) as never,
    );
    await threads.refreshHead();

    // t1 came from page 2 and must survive — load() would have dropped it.
    expect(threads.threads.map((t) => t.id)).toEqual(['t4', 't3', 't2', 't1']);
    // The paging cursor still describes the tail, so loadMore() keeps working.
    expect(threads.hasMoreThreads).toBe(true);
  });

  it('does not supersede the initial load when a sync event arrives first', async () => {
    const threads = useThreadsStore();
    let resolveInitial!: (value: ReturnType<typeof page>) => void;
    listThreads.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveInitial = resolve;
      }) as never,
    );

    const initialLoad = threads.load();
    expect(threads.threadLoadInFlight).toBe(true);

    await threads.refreshHead();
    expect(listThreads).toHaveBeenCalledTimes(1);

    resolveInitial(page([row('t1', '2026-08-10T00:00:00Z')], null, false));
    await initialLoad;

    expect(threads.threads.map((thread) => thread.id)).toEqual(['t1']);
    expect(threads.threadLoadInFlight).toBe(false);
  });

  it('lets a full load establish its cursor when refresh arrives with old rows cached', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(page([row('old', '2026-08-10T00:00:00Z')], { id: 'old' }, true) as never);
    await threads.load();
    let finish!: (value: ReturnType<typeof page>) => void;
    listThreads.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }) as never);
    const loading = threads.load();
    listThreads.mockResolvedValueOnce(page([row('new', '2026-08-11T00:00:00Z')], { id: 'head' }, false) as never);
    await threads.refreshHead();
    finish(page([row('new', '2026-08-11T00:00:00Z')], { id: 'new' }, true));
    await loading;
    await vi.waitFor(() => expect(threads.threadLoadInFlight).toBe(false));
    listThreads.mockResolvedValueOnce(page([], null, false) as never);
    await threads.loadMore();
    expect(listThreads).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: { id: 'new' } }));
  });

  it('keeps the last successful page and cursor when a full load fails', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(page([row('old', '2026-08-10T00:00:00Z')], { id: 'old' }, true) as never);
    await threads.load();
    listThreads.mockRejectedValueOnce(new Error('offline'));
    await threads.load();
    expect(threads.threads.map(thread => thread.id)).toEqual(['old']);
    expect(threads.hasMoreThreads).toBe(true);
  });

  it('publishes the first phone-sized page before smart summary metadata finishes', async () => {
    const threads = useThreadsStore();
    let resolveSummary!: (value: { total_count: number; buckets: never[] }) => void;
    listThreads.mockResolvedValueOnce(
      page([row('t1', '2026-08-10T00:00:00Z')], null, false) as never,
    );
    listSummary.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveSummary = resolve;
      }) as never,
    );

    const loading = threads.load();
    await vi.waitFor(() => expect(threads.threads.map((thread) => thread.id)).toEqual(['t1']));

    expect(listThreads).toHaveBeenCalledWith(expect.objectContaining({ limit: 40 }));
    expect(threads.threadLoadInFlight).toBe(true);
    resolveSummary({ total_count: 1, buckets: [] });
    await loading;
    expect(threads.threadLoadInFlight).toBe(false);
  });

  it('does not duplicate a thread that is already loaded', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(
      page([row('t1', '2026-07-31T10:00:00Z')], null, false) as never,
    );
    await threads.load();

    listThreads.mockResolvedValueOnce(
      page([row('t1', '2026-07-31T10:00:00Z')], null, false) as never,
    );
    await threads.refreshHead();

    expect(threads.threads.filter((t) => t.id === 't1')).toHaveLength(1);
  });

  it('renews the smart inbox summary so bucket counts stay current', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValue(
      page([row('t1', '2026-07-31T10:00:00Z')], null, false) as never,
    );
    listSummary.mockResolvedValueOnce({ total_count: 5, buckets: [] } as never);
    await threads.load();
    expect(threads.smartInboxTotalCount).toBe(5);

    // Checking off a mail refreshes via refreshHead — the folder-row counts
    // must follow, or they keep showing the pre-action numbers.
    listSummary.mockResolvedValueOnce({ total_count: 4, buckets: [] } as never);
    await threads.refreshHead();
    expect(threads.smartInboxTotalCount).toBe(4);
  });

  it('hides an earlier account summary as soon as the account changes', async () => {
    const accounts = useAccountsStore();
    accounts.accounts = [
      { id: 'acc-1', displayName: 'Eins', email: 'one@example.com', provider: 'imap', kind: 'email', color: '#111' },
      { id: 'acc-2', displayName: 'Zwei', email: 'two@example.com', provider: 'imap', kind: 'email', color: '#222' },
    ];
    accounts.setActiveAccount('acc-1');
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(
      page([row('t1', '2026-07-31T10:00:00Z')], null, false) as never,
    );
    listSummary.mockResolvedValueOnce({ total_count: 5, buckets: [] } as never);

    await threads.load();
    expect(threads.smartInboxTotalCount).toBe(5);

    accounts.setActiveAccount('acc-2');
    expect(threads.smartInboxTotalCount).toBe(0);
  });

  it('keeps the visible list when the refresh fails', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(
      page([row('t1', '2026-07-31T10:00:00Z')], null, false) as never,
    );
    await threads.load();

    listThreads.mockRejectedValueOnce(new Error('sync offline'));
    await threads.refreshHead();

    // load() blanks the list on error; a background refresh must not, or a
    // transient sync failure would clear a list the user is reading.
    expect(threads.threads.map((t) => t.id)).toEqual(['t1']);
  });

  it('removes an externally archived thread from the cached Inbox projection', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(
      page([row('t1', '2026-08-08T06:00:00Z')], null, false) as never,
    );
    await threads.load();
    expect(threads.visibleThreads.map((thread) => thread.id)).toEqual(['t1']);

    vi.mocked(backendGetThreads).mockResolvedValueOnce([{
      ...row('t1', '2026-08-08T06:00:00Z'),
      workflow_state: 'done',
      mailbox_names: JSON.stringify(['INBOX.Archive']),
    }] as never);
    await threads.refreshThreads(['t1']);

    expect(threads.threads[0]?.workflowState).toBe('done');
    expect(threads.visibleThreads.map((thread) => thread.id)).toEqual([]);
  });

  it('revalidates cached tail moves and deletes without resetting the mobile cursor', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(page([row('head', '2026-08-10')], { id: 'head' }, true) as never);
    await threads.load();
    listThreads.mockResolvedValueOnce(page([row('moved', '2026-08-09'), row('deleted', '2026-08-08'), row('tail', '2026-08-07')], { id: 'tail' }, true) as never);
    await threads.loadMore();
    threads.activeThreadId = 'tail';
    threads.selectedThreadIds = ['tail'];
    listThreads.mockResolvedValueOnce(page([row('head', '2026-08-10')], null, false) as never);
    vi.mocked(backendGetThreads).mockResolvedValueOnce([
      { ...row('moved', '2026-08-09'), workflow_state: 'done', mailbox_names: '["Archive"]' },
      row('tail', '2026-08-07'),
    ] as never);
    await threads.refreshHead({ revalidateCachedRows: true });
    expect(threads.visibleThreads.map(thread => thread.id)).toEqual(['head', 'tail']);
    expect(threads.activeThreadId).toBe('tail');
    expect(threads.selectedThreadIds).toEqual(['tail']);
    listThreads.mockResolvedValueOnce(page([], null, false) as never);
    await threads.loadMore();
    expect(listThreads).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: { id: 'tail' } }));
  });

  it('publishes a thread update batch once and retains unchanged rows', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(page([row('a', '2026-08-10'), row('b', '2026-08-09')], null, false) as never);
    await threads.load();
    const before = threads.threads;
    vi.mocked(backendGetThreads).mockResolvedValueOnce([row('a', '2026-08-10'), { ...row('b', '2026-08-09'), unread_count: 3 }] as never);
    await threads.refreshThreads(['a', 'b', 'b', 'unloaded']);
    expect(backendGetThreads).toHaveBeenLastCalledWith(['a', 'b']);
    expect(threads.threads[0]).toBe(before[0]);
    expect(threads.threads[1]?.unreadCount).toBe(3);
  });

  it('does not overwrite an optimistic row with a stale batch response', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(page([row('a', '2026-08-10')], null, false) as never);
    await threads.load();
    let finish!: (rows: never) => void;
    vi.mocked(backendGetThreads).mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const pending = threads.refreshThreads(['a']);
    threads.applyWorkflowStates([{ threadId: 'a', state: 'done' }]);
    finish([row('a', '2026-08-10')] as never);
    await pending;
    expect(threads.threads[0]?.workflowState).toBe('done');
  });

  it('keeps a directly opened done thread readable across a concurrent Inbox load', async () => {
    const threads = useThreadsStore();
    let resolveInbox!: (value: ReturnType<typeof page>) => void;
    listThreads.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveInbox = resolve;
      }) as never,
    );
    getThread.mockResolvedValueOnce({
      ...row('done-from-notification', '2026-08-08T06:00:00Z'),
      workflow_state: 'done',
      mailbox_names: JSON.stringify(['INBOX.Archive']),
    } as never);

    const inboxLoad = threads.load();
    await threads.ensureThreadLoaded('done-from-notification', {
      retainAcrossListLoads: true,
    });
    expect(threads.threads.map((thread) => thread.id)).toContain('done-from-notification');

    resolveInbox(page([row('inbox', '2026-08-09T06:00:00Z')], null, false));
    await inboxLoad;

    expect(threads.threads.map((thread) => thread.id)).toEqual([
      'inbox',
      'done-from-notification',
    ]);
    expect(threads.visibleThreads.map((thread) => thread.id)).toEqual(['inbox']);
    expect(threads.threads.find((thread) => thread.id === 'done-from-notification')?.workflowState)
      .toBe('done');
  });

  it('tells a thread the backend no longer has from one it could not load', async () => {
    const threads = useThreadsStore();
    getThread.mockRejectedValueOnce(new Error(JSON.stringify({ key: 'backend.errors.threadNotFound' })));
    await expect(threads.ensureThreadLoaded('gone')).resolves.toBe('missing');
    getThread.mockRejectedValueOnce(new Error(JSON.stringify({ key: 'backend.errors.threadPersistenceFailed' })));
    await expect(threads.ensureThreadLoaded('locked')).resolves.toBe('unavailable');
    getThread.mockResolvedValueOnce(row('found', '2026-08-08T06:00:00Z') as never);
    await expect(threads.ensureThreadLoaded('found')).resolves.toBe('loaded');
    await expect(threads.ensureThreadLoaded('found')).resolves.toBe('loaded');
    expect(getThread).toHaveBeenCalledTimes(3);
  });

  it('drops a cached thread when sync confirms that it no longer exists', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValueOnce(
      page([row('stale', '2026-08-08T06:00:00Z')], null, false) as never,
    );
    await threads.load();

    vi.mocked(backendGetThreads).mockResolvedValueOnce([]);
    await threads.refreshThreads(['stale']);

    expect(threads.threads).toEqual([]);
    expect(threads.activeThreadId).toBe('');
  });
});
