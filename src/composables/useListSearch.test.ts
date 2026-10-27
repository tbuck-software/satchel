import { effectScope, nextTick, reactive, shallowRef } from 'vue';
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
  listSearchScope,
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
  it('names the list the way the list request names it', () => {
    // The Inbox is inbox *and* todo with mail in the inbox folder; the
    // backend decides that from the same words the list uses.
    expect(listSearchScope(INBOX)).toEqual({
      contact_email: null, mailbox_kind: 'section', mailbox_value: 'inbox', workflow_state: null,
    });
    expect(listSearchScope({ ...INBOX, workflowFilter: 'snoozed', mailboxFilter: { kind: 'all', value: null } }))
      .toEqual({ contact_email: null, mailbox_kind: null, mailbox_value: null, workflow_state: 'snoozed' });
    expect(listSearchScope({ ...INBOX, mailboxFilter: { kind: 'folder', value: 'Projects' } }))
      .toMatchObject({ mailbox_kind: 'folder', mailbox_value: 'Projects' });
    // A contact list is the contact, whatever mailbox was open before.
    expect(listSearchScope({ ...INBOX, activeContactEmail: 'ada@example.org' }))
      .toEqual({ contact_email: 'ada@example.org', mailbox_kind: null, mailbox_value: null, workflow_state: null });
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
  const mailbox = shallowRef<ListSearchMailbox>(INBOX);

  function start() {
    scope = effectScope();
    return scope.run(() => useListSearch({ mailbox: () => mailbox.value }))!;
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
    mailbox.value = { ...INBOX };
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
    expect(harness.search).toHaveBeenLastCalledWith('invoice', expect.objectContaining({
      accountId: 'account-1',
      listScope: expect.objectContaining({ mailbox_kind: 'section', mailbox_value: 'inbox' }),
    }));
    // The store's copy of `b` is done: a row that left the Inbox drops out.
    expect(search.resultThreads.value.map((row) => row.id)).toEqual(['a']);
    expect(search.narrowed.value).toBe(true);

    store.setScope('all');
    await settle();
    // All mail is every account too, and no list.
    expect(harness.search).toHaveBeenLastCalledWith('invoice', expect.objectContaining({ accountId: undefined }));
    expect(harness.search.mock.lastCall?.[1]).not.toHaveProperty('listScope');
    expect(search.resultThreads.value.map((row) => row.id)).toEqual(['a', 'b']);
  });

  it('leaves the query as typed and names the list beside it', async () => {
    mailbox.value = { ...INBOX, workflowFilter: 'done', mailboxFilter: { kind: 'all', value: null } };
    const store = useListSearchStore();
    const search = start();

    store.setQuery('invoice');
    await settle();
    expect(harness.search).toHaveBeenCalledWith('invoice', expect.objectContaining({
      listScope: expect.objectContaining({ workflow_state: 'done', mailbox_kind: null }),
    }));
    expect(search.resultThreads.value.map((row) => row.id)).toEqual(['b']);
  });

  it('steps aside for a typed location', async () => {
    const store = useListSearchStore();
    const search = start();

    store.setQuery('invoice in:done');
    await settle();
    expect(search.narrowed.value).toBe(false);
    expect(harness.search.mock.lastCall?.[1]).not.toHaveProperty('listScope');
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

  it('pages only when the list asks: the backend already searched the list', async () => {
    harness.search.mockImplementation(async () => page(['a'], true));
    const store = useListSearchStore();
    const search = start();

    store.setQuery('invoice');
    for (let round = 0; round < 3; round += 1) await settle();
    expect(harness.search).toHaveBeenCalledTimes(1);
    expect(search.loading.value).toBe(false);

    search.loadMore();
    await settle();
    expect(harness.search).toHaveBeenCalledTimes(2);
    expect(harness.search).toHaveBeenLastCalledWith('invoice', expect.objectContaining({
      cursor: expect.objectContaining({ id: 'a' }),
      listScope: expect.objectContaining({ mailbox_value: 'inbox' }),
    }));
  });

  it('searches again when the list changes, not when it is recomputed', async () => {
    const store = useListSearchStore();
    start();

    store.setQuery('invoice');
    await settle();
    expect(harness.search).toHaveBeenCalledTimes(1);

    mailbox.value = { ...INBOX };
    await settle();
    expect(harness.search).toHaveBeenCalledTimes(1);

    mailbox.value = { ...INBOX, mailboxFilter: { kind: 'folder', value: 'Projects' } };
    await settle();
    expect(harness.search).toHaveBeenLastCalledWith('invoice', expect.objectContaining({
      listScope: expect.objectContaining({ mailbox_kind: 'folder', mailbox_value: 'Projects' }),
    }));
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
