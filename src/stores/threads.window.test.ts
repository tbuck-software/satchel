import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { backendListThreads } from '@/services/backend';
import { MAX_LOADED_THREADS } from '@/stores/threads/collection';
import { useThreadsStore } from './threads';

/*
 * PERF-05. `loadMore` was additive without a bound, so a research session that
 * paged to the end of the list dragged every projection along with it.
 */

vi.mock('@/services/backend', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/backend')>();
  return {
    ...actual,
    backendCommandsAvailable: () => true,
    backendListThreads: vi.fn(),
    backendListSmartInboxSummary: vi.fn(async () => null),
    backendListCachedThreads: vi.fn(async () => null),
  };
});

const listThreads = vi.mocked(backendListThreads);
/*
 * Larger than the store's own request size on purpose: the store appends
 * whatever a page returns, so serving 400 at a time reaches the 2000-row cap in
 * six calls instead of sixty. The first version drove 60 pages and took five
 * seconds, which is a test that fails when the machine is busy.
 */
const PAGE_SIZE = 400;

function backendThread(index: number) {
  return {
    id: `th-${String(index).padStart(5, '0')}`,
    account_id: 'acc-work',
    subject: `Betreff ${index}`,
    normalized_subject: `Betreff ${index}`,
    snippet: 'Vorschau',
    last_message_date: new Date(Date.UTC(2026, 0, 1) - index * 60_000).toISOString(),
    message_count: 1,
    workflow_state: 'inbox' as const,
    unread_count: 0,
    has_attachments: false,
    priority: 1,
    tags: [] as string[],
    participants: '[{"name":"Alice","email":"alice@example.com"}]',
    mailbox_names: '["INBOX"]',
  };
}

/** Serve consecutive pages, so paging never repeats a row. */
function servePages() {
  let served = 0;
  listThreads.mockReset().mockImplementation(async () => {
    const threads = Array.from({ length: PAGE_SIZE }, (_, offset) => backendThread(served + offset));
    served += PAGE_SIZE;
    const last = threads.at(-1)!;
    return {
      threads,
      next_cursor: { last_message_date: last.last_message_date, id: last.id },
      has_more: true,
    };
  });
}

async function pageThrough(store: ReturnType<typeof useThreadsStore>, pages: number) {
  for (let page = 0; page < pages; page += 1) {
    // The virtualizer sits at the tail while the user scrolls down.
    const tail = store.threads.at(-1);
    if (tail) store.setViewportStart(tail.id);
    await store.loadMore();
  }
}

describe('loaded thread window', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    servePages();
  });

  it('never grows past the cap, however far the list is paged', async () => {
    const store = useThreadsStore();
    await store.load();
    await pageThrough(store, 6);

    // Was 400 + 6 * 400 = 2800 before the trim.
    expect(store.threads).toHaveLength(MAX_LOADED_THREADS);
  });

  it('trims nothing while the viewport sits at the front', async () => {
    const store = useThreadsStore();
    await store.load();
    const first = store.threads[0]!.id;

    for (let page = 0; page < 6; page += 1) {
      store.setViewportStart(first);
      await store.loadMore();
    }

    expect(store.threads.length).toBeGreaterThan(MAX_LOADED_THREADS);
    expect(store.threads[0]!.id).toBe(first);
  });

  it('keeps the active thread and the selection across a trim', async () => {
    const store = useThreadsStore();
    await store.load();
    const pinnedActive = store.threads[1]!.id;
    const pinnedSelection = [store.threads[2]!.id, store.threads[3]!.id];
    store.selectThread(pinnedActive);
    store.setSelection(pinnedSelection);

    await pageThrough(store, 6);

    expect(store.activeThreadId).toBe(pinnedActive);
    expect(store.selectedThreadIds).toEqual(pinnedSelection);
    expect(store.threads.slice(0, 3).map((thread) => thread.id))
      .toEqual([pinnedActive, ...pinnedSelection]);
  });

  it('leaves the tail cursor alone so paging continues', async () => {
    const store = useThreadsStore();
    await store.load();
    await pageThrough(store, 6);

    const lastBefore = store.threads.at(-1)!.id;
    expect(store.hasMoreThreads).toBe(true);

    await pageThrough(store, 1);

    expect(store.threads).toHaveLength(MAX_LOADED_THREADS);
    expect(store.threads.at(-1)!.id).not.toBe(lastBefore);
  });
  it('can page back into trimmed rows and forward again without growing the window', async () => {
    const all = Array.from({ length: 3200 }, (_, index) => {
      const row = backendThread(index);
      return { ...row, sort_date: row.last_message_date,
        last_message_date: new Date(Date.parse(row.last_message_date) + 86_400_000).toISOString() };
    });
    listThreads.mockImplementation(async (input) => {
      const cursor = input?.before_cursor ?? input?.cursor;
      const index = cursor ? all.findIndex(row => row.id === cursor.id) : -1;
      const backwards = !!input?.before_cursor;
      const start = backwards ? Math.max(0, index - PAGE_SIZE) : index + 1;
      const end = backwards ? index : start + PAGE_SIZE;
      const threads = all.slice(start, end);
      const hasMore = backwards ? start > 0 : end < all.length;
      const last = backwards ? threads[0]! : threads.at(-1)!;
      return { threads, has_more: hasMore, next_cursor: hasMore
        ? { id: last.id, last_message_date: last.last_message_date } : null };
    });
    const store = useThreadsStore();
    await store.load();
    await pageThrough(store, 6);
    expect(store.threads.some(row => row.id === 'th-00400')).toBe(false);
    const firstWindowRow = store.threads.find(row => row.id !== store.activeThreadId)!;
    const anchor = store.threads[10]!;
    store.setViewportStart(anchor.id);
    await store.loadPrevious();
    expect(listThreads).toHaveBeenLastCalledWith(expect.objectContaining({
      before_cursor: expect.objectContaining({ id: firstWindowRow.id,
        last_message_date: all.find(row => row.id === firstWindowRow.id)!.sort_date }),
    }));
    expect(store.find('th-00401')).toBeDefined();
    expect(store.find(anchor.id)).toBeDefined();
    expect(store.threads).toHaveLength(MAX_LOADED_THREADS);
    await pageThrough(store, 1);
    expect(store.threads).toHaveLength(MAX_LOADED_THREADS);
    expect(store.find('th-02799')).toBeDefined();
  });

  it('reports reverse page failures and retries the failed direction only on request', async () => {
    const store = useThreadsStore();
    await store.load();
    await pageThrough(store, 6);
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    listThreads.mockRejectedValueOnce(new Error('offline'));
    await store.loadPrevious();
    expect(store.loadMoreFailed).toBe(true);
    expect(store.loadPreviousFailed).toBe(true);
    const calls = listThreads.mock.calls.length;
    await store.loadPrevious();
    expect(listThreads).toHaveBeenCalledTimes(calls);
    await store.retryLoadMore();
    expect(listThreads).toHaveBeenLastCalledWith(expect.objectContaining({ before_cursor: expect.any(Object) }));
    expect(store.loadMoreFailed).toBe(false);
    error.mockRestore();
  });

  it('does not let an obsolete reverse request clear a new scope page load', async () => {
    const store = useThreadsStore();
    await store.load();
    await pageThrough(store, 6);
    let resolveOld!: (value: Awaited<ReturnType<typeof backendListThreads>>) => void;
    listThreads.mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; }));
    const oldRequest = store.loadPrevious();
    await store.load();
    let resolveNew!: (value: Awaited<ReturnType<typeof backendListThreads>>) => void;
    listThreads.mockImplementationOnce(() => new Promise(resolve => { resolveNew = resolve; }));
    const newRequest = store.loadMore();
    expect(store.loadingMoreThreads).toBe(true);
    resolveOld({ threads: [], next_cursor: null, has_more: false });
    await oldRequest;
    expect(store.loadingMoreThreads).toBe(true);
    resolveNew({ threads: [], next_cursor: null, has_more: false });
    await newRequest;
    expect(store.loadingMoreThreads).toBe(false);
  });

});
