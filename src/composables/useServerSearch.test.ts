import { nextTick, reactive, ref } from 'vue';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ServerHitEntry } from './useServerSearch';

const harness = vi.hoisted(() => ({
  search: vi.fn(),
  fetch: vi.fn(),
  native: vi.fn(() => true),
  accounts: { isVisible: (_id: string): boolean => true, accounts: [] as Array<{ id: string; provider: string }> },
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
  harness.search.mockReset();
  harness.accounts = reactive({
    isVisible: (_id: string): boolean => true,
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

  it('keeps resting accounts out of shared server search but allows an explicit account', async () => {
    harness.accounts.isVisible = id => id !== 'gmail-1';
    const { accountFilter, actions } = setup();
    await actions.searchServer();
    expect(harness.search).toHaveBeenCalledTimes(1);
    expect(harness.search).toHaveBeenCalledWith({ accountId: 'outlook-1', query: 'invoice' });
    accountFilter.value = 'gmail-1';
    await nextTick();
    await actions.searchServer();
    expect(harness.search).toHaveBeenLastCalledWith({ accountId: 'gmail-1', query: 'invoice' });
  });

  it('does not call native search when the runtime has no native capability', async () => {
    harness.native.mockReturnValue(false);
    const { actions } = setup();

    expect(actions.canSearchServer.value).toBe(false);
    await actions.searchServer();

    expect(harness.search).not.toHaveBeenCalled();
    expect(actions.serverQueried.value).toBe(false);
  });

  it('keeps successful hits when another account fails and retries without losing them', async () => {
    harness.search.mockRejectedValueOnce(new Error('network is unreachable'));
    const { actions } = setup();
    await actions.searchServer();
    expect(actions.serverHits.value.map(entry => entry.accountId)).toEqual(['outlook-1']);
    expect(actions.serverError.value).toContain('Some accounts could not be searched');
    expect(actions.serverError.value).toContain('as soon as the network is back');
    expect(actions.serverQueried.value).toBe(true);

    let retry!: (hits: ReturnType<typeof hit>[]) => void;
    harness.search.mockReturnValueOnce(new Promise(resolve => { retry = resolve; }));
    const searching = actions.searchServer();
    expect(actions.serverHits.value.map(entry => entry.accountId)).toEqual(['outlook-1']);
    retry([hit('recovered')]);
    await searching;
    expect(actions.serverError.value).toBe('');
    expect(actions.serverHits.value.map(entry => entry.accountId)).toEqual(['outlook-1', 'gmail-1']);
    expect(harness.search).toHaveBeenCalledTimes(3);
  });

  it('distinguishes complete failure from a successful empty search', async () => {
    harness.search.mockRejectedValue(new Error('unavailable'));
    const { actions } = setup();
    await actions.searchServer();
    expect(actions.serverError.value).toBe('Server search failed. Check the connection and the sign-in.');
    expect(actions.serverQueried.value).toBe(false);
    expect(actions.serverHits.value).toEqual([]);
    harness.search.mockResolvedValue([]);
    await actions.searchServer();
    expect(actions.serverQueried.value).toBe(true);
    expect(actions.serverError.value).toBe('');
    expect(actions.serverHits.value).toEqual([]);
  });

  it('removes only the selected account hit when remote IDs collide', async () => {
    harness.search.mockResolvedValue([hit('same-id')]);
    const { actions } = setup();
    await actions.searchServer();
    const opening = actions.openServerHit(actions.serverHits.value[0]!);
    expect(actions.fetchingHitId.value).toBe(JSON.stringify(['gmail-1', 'same-id']));
    await opening;
    expect(actions.serverHits.value).toEqual([{ accountId: 'outlook-1', hit: hit('same-id') }]);
  });

  it('surfaces a server-search failure and clears loading', async () => {
    harness.search.mockRejectedValue(new Error('mime parse failed at part 3'));
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
    expect(actions.fetchingHitId.value).toBe(JSON.stringify(['gmail-1', 'remote-1']));

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

  it.each(['query', 'accountFilter', 'accounts'] as const)('ignores late mixed responses after %s changes and a newer search finishes', async (change) => {
    let finish!: (hits: ReturnType<typeof hit>[]) => void;
    let fail!: (error: Error) => void;
    harness.search.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }))
      .mockReturnValueOnce(new Promise((_resolve, reject) => { fail = reject; }));
    const { query, accountFilter, actions } = setup();
    const oldSearch = actions.searchServer();
    if (change === 'query') query.value = 'receipt';
    if (change === 'accountFilter') accountFilter.value = 'gmail-1';
    if (change === 'accounts') harness.accounts.accounts = [account('new-account', 'gmail')];
    await actions.searchServer();
    const currentHits = [...actions.serverHits.value];
    finish([hit('stale')]);
    fail(new Error('network is unreachable'));
    await oldSearch;
    expect(actions.serverHits.value).toEqual(currentHits);
    expect(actions.serverError.value).toBe('');
    expect(actions.serverQueried.value).toBe(true);
    expect(actions.serverLoading.value).toBe(false);
    // A stale failure must not narrow the next search to obsolete failed accounts.
    harness.search.mockClear();
    await actions.searchServer();
    expect(harness.search).toHaveBeenCalledTimes(change === 'query' ? 2 : 1);
  });

  it('keeps a newer search busy when stale success and failure responses finish', async () => {
    let finishOld!: (hits: ReturnType<typeof hit>[]) => void;
    let failOld!: (error: Error) => void;
    let finishNew!: (hits: ReturnType<typeof hit>[]) => void;
    harness.search.mockReturnValueOnce(new Promise(resolve => { finishOld = resolve; }))
      .mockReturnValueOnce(new Promise((_resolve, reject) => { failOld = reject; }))
      .mockReturnValueOnce(new Promise(resolve => { finishNew = resolve; }));
    const { query, actions } = setup();
    const oldSearch = actions.searchServer();
    query.value = 'receipt';
    const newSearch = actions.searchServer();
    finishOld([hit('stale')]);
    failOld(new Error('network is unreachable'));
    await oldSearch;
    expect(actions.serverLoading.value).toBe(true);
    expect(actions.serverError.value).toBe('');
    expect(actions.serverHits.value).toEqual([]);
    finishNew([hit('current')]);
    await newSearch;
    expect(actions.serverLoading.value).toBe(false);
    expect(actions.serverHits.value[0]!.hit.remote_id).toBe('current');
  });

  it('does not let a stale hit download clear a newer account-scoped fetch guard', async () => {
    let finishOld!: (value: { message_id: string; thread_id: string }) => void;
    let finishNew!: (value: { message_id: string; thread_id: string }) => void;
    harness.fetch.mockReturnValueOnce(new Promise(resolve => { finishOld = resolve; }))
      .mockReturnValueOnce(new Promise(resolve => { finishNew = resolve; }));
    const { query, actions, selected } = setup();
    const oldOpening = actions.openServerHit({ accountId: 'gmail-1', hit: hit('same') });
    query.value = 'receipt';
    const newOpening = actions.openServerHit({ accountId: 'outlook-1', hit: hit('same') });
    finishOld({ message_id: 'old', thread_id: 'old' });
    await oldOpening;
    expect(selected).toEqual([]);
    expect(actions.fetchingHitId.value).toBe(JSON.stringify(['outlook-1', 'same']));
    finishNew({ message_id: 'new', thread_id: 'new' });
    await newOpening;
    expect(selected).toEqual(['new']);
    expect(actions.fetchingHitId.value).toBe('');
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
