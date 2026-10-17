import { nextTick, reactive, ref } from 'vue';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ServerHitEntry } from './useServerSearch';

const harness = vi.hoisted(() => ({
  search: vi.fn(),
  fetch: vi.fn(),
  native: vi.fn(() => true),
  accounts: { accounts: [] as Array<{ id: string; provider: string }> },
  threads: {
    ensureThreadLoaded: vi.fn(),
    selectThread: vi.fn(),
  },
  pushRecent: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('@/services/backend', () => ({
  backendServerSearch: harness.search,
  backendFetchServerSearchHit: harness.fetch,
  backendCommandsAvailable: () => harness.native(),
}));
vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => harness.accounts,
}));
vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => harness.threads,
}));
vi.mock('@/composables/useRecentSearches', () => ({
  useRecentSearches: () => ({ push: harness.pushRecent }),
}));
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ error: harness.toastError }),
}));

import { useServerSearch } from './useServerSearch';

function hit(remoteId: string) {
  return {
    remote_id: remoteId,
    subject: `Subject ${remoteId}`,
    snippet: `Snippet ${remoteId}`,
    from: 'sender@example.com',
    date: '2026-08-30T10:00:00Z',
  };
}

function account(id: string, provider: string) {
  return { id, provider };
}

function setup() {
  const query = ref('invoice');
  const accountFilter = ref('');
  const selected: string[] = [];
  const actions = useServerSearch({
    query,
    accountFilter,
    onThreadSelected: (threadId) => selected.push(threadId),
  });
  return { query, accountFilter, selected, actions };
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.accounts = reactive({
    accounts: [
      account('gmail-1', 'gmail'),
      account('outlook-1', 'outlook'),
      account('imap-1', 'imap'),
    ],
  });
  harness.native.mockReturnValue(true);
  harness.search.mockImplementation(async ({ accountId }: { accountId: string }) => [hit(accountId)]);
  harness.fetch.mockResolvedValue({ message_id: 'message-1', thread_id: 'thread-1' });
  harness.threads.ensureThreadLoaded.mockResolvedValue(undefined);
});

describe('useServerSearch', () => {
  it('searches Gmail and Outlook in parallel while honoring account scope', async () => {
    const { accountFilter, actions } = setup();

    expect(actions.canSearchServer.value).toBe(true);
    await actions.searchServer();

    expect(harness.search).toHaveBeenCalledTimes(2);
    expect(harness.search).toHaveBeenCalledWith({ accountId: 'gmail-1', query: 'invoice' });
    expect(harness.search).toHaveBeenCalledWith({ accountId: 'outlook-1', query: 'invoice' });
    expect(actions.serverHits.value.map((entry) => entry.accountId)).toEqual([
      'gmail-1',
      'outlook-1',
    ]);
    expect(actions.serverQueried.value).toBe(true);
    expect(actions.serverLoading.value).toBe(false);

    accountFilter.value = 'outlook-1';
    await nextTick();
    expect(actions.serverHits.value).toEqual([]);
    expect(actions.serverQueried.value).toBe(false);

    await actions.searchServer();
    expect(harness.search).toHaveBeenCalledTimes(3);
    expect(harness.search).toHaveBeenLastCalledWith({ accountId: 'outlook-1', query: 'invoice' });
  });

  it('does not call native search when the runtime has no native capability', async () => {
    harness.native.mockReturnValue(false);
    const { actions } = setup();

    expect(actions.canSearchServer.value).toBe(false);
    await actions.searchServer();

    expect(harness.search).not.toHaveBeenCalled();
    expect(actions.serverQueried.value).toBe(false);
  });

  it('surfaces a server-search failure and clears loading', async () => {
    harness.search.mockRejectedValueOnce(new Error('mime parse failed at part 3'));
    const { actions } = setup();

    await actions.searchServer();

    expect(actions.serverError.value).toBe('Server search failed. Check the connection and the sign-in.');
    expect(actions.serverLoading.value).toBe(false);
    expect(actions.serverQueried.value).toBe(false);
  });

  // "Check the connection and the sign-in" covers three different problems and
  // helps with none of them: one is waited out, one is re-authorized, one is
  // the network.
  it.each([
    ['network is unreachable', 'as soon as the network is back'],
    ['oauth refresh failed: invalid_grant', 'sign in again'],
    ['429 Too Many Requests', 'turning requests away'],
  ])('tells the reader what to do about %s', async (chain, expected) => {
    harness.search.mockRejectedValueOnce(new Error(chain));
    const { actions } = setup();

    await actions.searchServer();

    expect(actions.serverError.value).toContain(expected);
  });

  it('fetches a hit, removes it, records the query, and selects its thread', async () => {
    const { selected, actions } = setup();
    await actions.searchServer();
    const entry = actions.serverHits.value[0] as ServerHitEntry;

    await actions.openServerHit(entry);

    expect(harness.fetch).toHaveBeenCalledWith({ accountId: entry.accountId, remoteId: entry.hit.remote_id });
    expect(actions.serverHits.value).toHaveLength(1);
    expect(actions.serverHits.value.every((candidate) => candidate.hit.remote_id !== entry.hit.remote_id)).toBe(true);
    expect(harness.pushRecent).toHaveBeenCalledWith('invoice');
    expect(harness.threads.ensureThreadLoaded).toHaveBeenCalledWith('thread-1');
    expect(harness.threads.selectThread).toHaveBeenCalledWith('thread-1');
    expect(selected).toEqual(['thread-1']);
    expect(actions.fetchingHitId.value).toBe('');
  });

  it('allows only one hit fetch at a time and reports fetch failures', async () => {
    const { actions } = setup();
    const pending = new Promise<never>(() => {});
    harness.fetch.mockReturnValueOnce(pending);
    const entry: ServerHitEntry = { accountId: 'gmail-1', hit: hit('remote-1') };

    void actions.openServerHit(entry);
    await nextTick();
    await actions.openServerHit(entry);

    expect(harness.fetch).toHaveBeenCalledOnce();
    expect(actions.fetchingHitId.value).toBe('remote-1');

    harness.fetch.mockReset();
    harness.fetch.mockRejectedValueOnce(new Error('gone'));
    // A new composable keeps the test independent from the intentionally
    // unresolved request above.
    const retry = setup().actions;
    await retry.openServerHit(entry);

    expect(harness.toastError).toHaveBeenCalledWith('The message could not be loaded from the server.');
    expect(retry.fetchingHitId.value).toBe('');
  });

  it('drops server results when the query changes', async () => {
    const { query, actions } = setup();
    await actions.searchServer();
    expect(actions.serverHits.value).not.toHaveLength(0);

    query.value = 'receipt';
    await nextTick();

    expect(actions.serverHits.value).toEqual([]);
    expect(actions.serverError.value).toBe('');
    expect(actions.serverQueried.value).toBe(false);
  });

  it('drops a response when the searchable account scope changes', async () => {
    let finishSearch!: (hits: ReturnType<typeof hit>[]) => void;
    harness.accounts.accounts = [account('shared-account', 'gmail')];
    harness.search.mockReturnValueOnce(
      new Promise((resolve) => { finishSearch = resolve; }),
    );
    const { actions } = setup();

    const searching = actions.searchServer();
    await vi.waitFor(() => {
      expect(harness.search).toHaveBeenCalledWith({ accountId: 'shared-account', query: 'invoice' });
    });

    harness.accounts.accounts = [account('shared-account', 'outlook')];

    expect(actions.serverLoading.value).toBe(false);
    expect(actions.serverHits.value).toEqual([]);
    expect(actions.serverQueried.value).toBe(false);

    finishSearch([hit('stale-gmail-hit')]);
    await searching;

    expect(actions.serverHits.value).toEqual([]);
    expect(actions.serverQueried.value).toBe(false);
    expect(actions.serverError.value).toBe('');
  });

  it('drops a search response when the query changes before watchers flush', async () => {
    let finishSearch!: (hits: ReturnType<typeof hit>[]) => void;
    harness.accounts.accounts = [account('gmail-1', 'gmail')];
    harness.search.mockReturnValueOnce(
      new Promise((resolve) => { finishSearch = resolve; }),
    );
    const { query, actions } = setup();

    const searching = actions.searchServer();
    await vi.waitFor(() => {
      expect(harness.search).toHaveBeenCalledWith({ accountId: 'gmail-1', query: 'invoice' });
    });

    query.value = 'receipt';
    finishSearch([hit('stale-invoice-hit')]);
    await searching;

    expect(actions.serverHits.value).toEqual([]);
    expect(actions.serverQueried.value).toBe(false);
    expect(actions.serverLoading.value).toBe(false);
  });

  it('does not select a fetched thread after the query changes during local loading', async () => {
    let finishLoading!: () => void;
    harness.threads.ensureThreadLoaded.mockReturnValueOnce(
      new Promise<void>((resolve) => { finishLoading = resolve; }),
    );
    const { query, selected, actions } = setup();
    const entry: ServerHitEntry = { accountId: 'gmail-1', hit: hit('remote-1') };

    const opening = actions.openServerHit(entry);
    await vi.waitFor(() => {
      expect(harness.threads.ensureThreadLoaded).toHaveBeenCalledWith('thread-1');
    });
    query.value = 'receipt';
    finishLoading();
    await opening;

    expect(harness.threads.selectThread).not.toHaveBeenCalled();
    expect(selected).toEqual([]);
  });
});
