import { effectScope, nextTick, reactive } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Thread } from '@/types/mail';

const harness = vi.hoisted(() => ({
  search: vi.fn(),
  getThreads: vi.fn(),
  threads: null as { threads: Thread[] } | null,
  accounts: null as { activeAccountId: string; accounts: unknown[] } | null,
}));

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: () => true,
  backendSearchMessageHits: harness.search,
  backendGetThreads: harness.getThreads,
}));
vi.mock('@/composables/useMailEvents', () => ({ useMailEvents: vi.fn() }));
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => harness.threads }));
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => harness.accounts }));

import {
  LIST_SEARCH_MAX_FILL_PAGES,
  mailboxSearchOperator,
  namesLocation,
  threadMatchesListSearch,
  useListSearch,
  type ListSearchMailbox,
} from './useListSearch';
import { SEARCH_DEBOUNCE_MS } from './useSearch';
import { useListSearchStore } from '@/stores/listSearch';

function thread(id: string, workflowState: Thread['workflowState'], mailboxNames = ['INBOX']): Thread {
  return { id, accountId: 'account-1', workflowState, mailboxNames, participants: [] } as unknown as Thread;
}

function hit(threadId: string) {
  return {
    id: `m-${threadId}`,
    thread_id: threadId,
    subject: threadId,
    snippet: '',
    from_name: 'Ada',
    from_address: 'ada@example.org',
    date: '2026-10-01T10:00:00Z',
    is_read: true,
    folder_type: 'inbox',
    workflow_state: 'inbox',
  };
}

function page(threadIds: string[], hasMore = false) {
  return {
    hits: threadIds.map(hit),
    warnings: [],
    has_more: hasMore,
    next_cursor: hasMore ? { rank_cost: 0, date: '2026-10-01T10:00:00Z', id: threadIds.at(-1) } : null,
  };
}

const INBOX: ListSearchMailbox = {
  accountId: 'account-1',
  workflowFilter: 'all',
  mailboxFilter: { kind: 'section', value: 'inbox' },
  activeContactEmail: '',
};

describe('mailbox scope helpers', () => {
  it('names the backend operator only where it says exactly the mailbox', () => {
    expect(mailboxSearchOperator({ ...INBOX, workflowFilter: 'snoozed', mailboxFilter: { kind: 'all', value: null } }))
      .toBe('in:snoozed');
    expect(mailboxSearchOperator({ ...INBOX, mailboxFilter: { kind: 'section', value: 'sent' } })).toBe('in:sent');
    expect(mailboxSearchOperator({ ...INBOX, mailboxFilter: { kind: 'section', value: 'archive' } })).toBe('in:done');
    // The Inbox list is inbox *and* todo; one operator cannot say that.
    expect(mailboxSearchOperator(INBOX)).toBeNull();
    expect(mailboxSearchOperator({ ...INBOX, mailboxFilter: { kind: 'folder', value: 'Projects' } })).toBeNull();
    expect(mailboxSearchOperator({ ...INBOX, activeContactEmail: 'ada@example.org' })).toBeNull();
  });

  it('matches threads the way the list does', () => {
    expect(threadMatchesListSearch(thread('a', 'inbox'), INBOX)).toBe(true);
    expect(threadMatchesListSearch(thread('b', 'todo'), INBOX)).toBe(true);
    expect(threadMatchesListSearch(thread('c', 'done'), INBOX)).toBe(false);
    expect(threadMatchesListSearch({ ...thread('d', 'inbox'), accountId: 'account-2' }, INBOX)).toBe(false);
    const folder = { ...INBOX, mailboxFilter: { kind: 'folder' as const, value: 'Projects' } };
    expect(threadMatchesListSearch(thread('e', 'inbox', ['Projects']), folder)).toBe(true);
    expect(threadMatchesListSearch(thread('f', 'inbox', ['INBOX']), folder)).toBe(false);
  });

  it('lets a typed in: operator name its own place', () => {
    expect(namesLocation('invoice in:sent')).toBe(true);
    expect(namesLocation('in:')).toBe(false);
    expect(namesLocation('login')).toBe(false);
  });
});

describe('useListSearch', () => {
  let scope: ReturnType<typeof effectScope>;
  let mailbox: ListSearchMailbox;

  function start() {
    scope = effectScope();
    return scope.run(() => useListSearch({ mailbox: () => mailbox }))!;
  }

  async function settle() {
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS + 1);
    await nextTick();
    await vi.runOnlyPendingTimersAsync();
    await nextTick();
  }

  beforeEach(() => {
    vi.useFakeTimers();
    setActivePinia(createPinia());
    mailbox = { ...INBOX };
    harness.threads = reactive({ threads: [thread('a', 'inbox'), thread('b', 'done')] });
    harness.accounts = reactive({ activeAccountId: 'account-1', accounts: [] });
    harness.search.mockReset().mockResolvedValue(page(['a', 'b']));
    harness.getThreads.mockReset().mockResolvedValue([]);
  });

  afterEach(() => {
    scope.stop();
    vi.useRealTimers();
  });

  it('waits for typing to pause before it searches once', async () => {
    const store = useListSearchStore();
    start();

    store.setQuery('inv');
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS / 2);
    store.setQuery('invoice');
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS / 2);
    expect(harness.search).not.toHaveBeenCalled();

    await settle();
    expect(harness.search).toHaveBeenCalledTimes(1);
    expect(harness.search).toHaveBeenCalledWith('invoice', expect.objectContaining({ accountId: 'account-1' }));
  });

  it('narrows to the mailbox by default and widens to all mail on request', async () => {
    const store = useListSearchStore();
    const search = start();

    store.setQuery('invoice');
    await settle();
    expect(search.resultThreads.value.map((row) => row.id)).toEqual(['a']);
    expect(search.narrowed.value).toBe(true);

    store.setScope('all');
    await settle();
    // All mail is every account too.
    expect(harness.search).toHaveBeenLastCalledWith('invoice', expect.objectContaining({ accountId: undefined }));
    expect(search.resultThreads.value.map((row) => row.id)).toEqual(['a', 'b']);
  });

  it('hands the operator to the backend where one names the mailbox', async () => {
    mailbox = { ...INBOX, workflowFilter: 'done', mailboxFilter: { kind: 'all', value: null } };
    const store = useListSearchStore();
    const search = start();

    store.setQuery('invoice');
    await settle();
    expect(harness.search).toHaveBeenCalledWith('in:done invoice', expect.anything());
    expect(search.resultThreads.value.map((row) => row.id)).toEqual(['b']);
  });

  it('steps aside for a typed location', async () => {
    const store = useListSearchStore();
    const search = start();

    store.setQuery('invoice in:done');
    await settle();
    expect(search.narrowed.value).toBe(false);
    expect(search.resultThreads.value.map((row) => row.id)).toEqual(['a', 'b']);
  });

  it('resolves hits outside the loaded page and keeps them', async () => {
    harness.search.mockResolvedValue(page(['a', 'z']));
    const store = useListSearchStore();
    start();

    store.setQuery('invoice');
    await settle();
    expect(harness.getThreads).toHaveBeenCalledWith(['a', 'z']);
  });

  it('pages on its own while a narrowed list is short, but not without end', async () => {
    harness.search.mockImplementation(async () => page(['b'], true));
    const store = useListSearchStore();
    const search = start();

    store.setQuery('invoice');
    for (let round = 0; round < LIST_SEARCH_MAX_FILL_PAGES + 3; round += 1) await settle();

    // The first page plus the bounded fill.
    expect(harness.search).toHaveBeenCalledTimes(1 + LIST_SEARCH_MAX_FILL_PAGES);
    expect(search.resultThreads.value).toEqual([]);
    expect(search.loading.value).toBe(false);

    // The list's own end-of-scroll request does not reopen the walk.
    search.loadMore();
    await settle();
    expect(harness.search).toHaveBeenCalledTimes(1 + LIST_SEARCH_MAX_FILL_PAGES);
  });

  it('clears query, scope and the open hit together', () => {
    const store = useListSearchStore();
    store.open('from:ada@example.org', 'all');
    store.openThreadId = 'a';
    expect(store.active).toBe(true);

    store.clear();
    expect(store.query).toBe('');
    expect(store.scope).toBe('mailbox');
    expect(store.openThreadId).toBe('');
    expect(store.active).toBe(false);
  });
});
