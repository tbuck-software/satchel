import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';

const searchMock = vi.fn();
const nativeMock = vi.fn(() => true);
const availableMock = vi.fn(() => true);

vi.mock('@/services/backend', () => ({
  backendSearchMessageHits: (...args: unknown[]) => searchMock(...args),
  backendCommandsAvailable: () => availableMock(),
  backendSupportsNativeCommands: () => nativeMock(),
}));

vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => ({ activeAccountId: 'acc-1' }),
}));

vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => ({ threads: [] }),
}));

import { useSearch } from './useSearch';

function backendHit(overrides: Record<string, unknown> = {}) {
  return {
    id: 'msg-1',
    thread_id: 'thread-1',
    subject: 'Rechnung 2023',
    snippet: 'Anbei die Rechnung',
    from_name: 'Billing',
    from_address: 'billing@example.com',
    date: '2026-07-01T10:00:00Z',
    is_read: false,
    ...overrides,
  };
}

async function flushDebounce() {
  await nextTick();
  await vi.advanceTimersByTimeAsync(300);
  await nextTick();
}

describe('useSearch', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
    searchMock.mockReset();
    searchMock.mockResolvedValue({ hits: [backendHit()], warnings: [] });
    nativeMock.mockReturnValue(true);
    availableMock.mockReturnValue(true);
  });

  it('keeps one request in flight and skips superseded queued searches', async () => {
    let finish: (response: unknown) => void = () => {};
    searchMock.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const { query, results } = useSearch();
    query.value = 'm';
    await flushDebounce();
    query.value = 'ma';
    await flushDebounce();
    query.value = 'map';
    await flushDebounce();
    expect(searchMock).toHaveBeenCalledTimes(1);
    finish({ hits: [backendHit({ id: 'old' })], warnings: [] });
    await flushDebounce();
    expect(searchMock.mock.calls.map((call) => call[0])).toEqual(['m', 'map']);
    expect(results.value[0]?.messageId).toBe('msg-1');
  });

  it('uses the command transport in a browser mock for operator queries', async () => {
    nativeMock.mockReturnValue(false);
    const { query, results } = useSearch();
    query.value = 'from:billing@example.com has:attachment';
    await flushDebounce();
    expect(searchMock).toHaveBeenCalledWith(query.value, { accountId: 'acc-1' });
    expect(results.value[0]?.messageId).toBe('msg-1');
  });

  it('queries the backend, not the in-memory thread list', async () => {
    const { query, results } = useSearch();
    query.value = 'rechnung';
    await flushDebounce();

    expect(searchMock).toHaveBeenCalledWith('rechnung', { accountId: 'acc-1' });
    expect(results.value).toHaveLength(1);
    expect(results.value[0]!.threadId).toBe('thread-1');
  });

  it('passes operator syntax through untouched (backend owns the parser)', async () => {
    const { query } = useSearch();
    query.value = 'from:billing@example.com has:attachment';
    await flushDebounce();

    expect(searchMock).toHaveBeenCalledWith(
      'from:billing@example.com has:attachment',
      { accountId: 'acc-1' },
    );
  });

  it('debounces so one request fires per burst of typing', async () => {
    const { query } = useSearch();
    for (const value of ['r', 're', 'rec', 'rech']) {
      query.value = value;
      await nextTick();
    }
    await flushDebounce();

    expect(searchMock).toHaveBeenCalledTimes(1);
    expect(searchMock).toHaveBeenCalledWith('rech', { accountId: 'acc-1' });
  });

  it('does not flash a loading indicator for a fast search', async () => {
    let resolveSearch!: (value: unknown) => void;
    searchMock.mockReturnValueOnce(new Promise((resolve) => { resolveSearch = resolve; }));
    const { query, loading, showLoading } = useSearch();

    query.value = 'rechnung';
    await nextTick();
    expect(loading.value).toBe(true);
    expect(showLoading.value).toBe(false);

    await vi.advanceTimersByTimeAsync(159);
    expect(showLoading.value).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(showLoading.value).toBe(true);

    resolveSearch({ hits: [backendHit()], warnings: [] });
    await vi.advanceTimersByTimeAsync(0);
    expect(loading.value).toBe(false);
    expect(showLoading.value).toBe(false);
  });

  it('clears results for an empty query without hitting the backend', async () => {
    const { query, results } = useSearch();
    query.value = 'rechnung';
    await flushDebounce();
    expect(results.value).toHaveLength(1);

    query.value = '   ';
    await flushDebounce();

    expect(results.value).toHaveLength(0);
    expect(searchMock).toHaveBeenCalledTimes(1);
  });

  it('splits marked snippets into segments and strips the markers', async () => {
    searchMock.mockResolvedValueOnce({
      hits: [backendHit({ snippet: 'Anbei die \u0001Rechnung\u0002 für März' })],
      warnings: [],
    });
    const { query, results } = useSearch();
    query.value = 'rechnung';
    await flushDebounce();

    expect(results.value[0]!.snippetSegments).toEqual([
      { text: 'Anbei die ', marked: false },
      { text: 'Rechnung', marked: true },
      { text: ' für März', marked: false },
    ]);
    expect(results.value[0]!.snippet).toBe('Anbei die Rechnung für März');
  });

  it('translates ignored-operator warnings into English notes', async () => {
    searchMock.mockResolvedValueOnce({
      hits: [backendHit()],
      warnings: [
        { operator: 'in', value: 'xyz' },
        { operator: 'after', value: 'gestern' },
      ],
    });
    const { query, warnings } = useSearch();
    query.value = 'in:xyz after:gestern rechnung';
    await flushDebounce();

    expect(warnings.value).toEqual([
      '“in:xyz” is ignored – unknown value.',
      '“after:gestern” is ignored – give the date as YYYY-MM-DD or relative (7d, 2w, yesterday).',
    ]);

    query.value = '';
    await flushDebounce();
    expect(warnings.value).toEqual([]);
  });

  it('surfaces backend failures instead of pretending there are no hits', async () => {
    searchMock.mockRejectedValueOnce(new Error('db locked'));
    const { query, results, errorText } = useSearch();
    query.value = 'rechnung';
    await flushDebounce();

    expect(results.value).toHaveLength(0);
    expect(errorText.value).toBe('db locked');
  });

  it('discards the active response when the query changes', async () => {
    let resolveFirst: ((value: unknown) => void) | undefined;
    searchMock.mockImplementationOnce(
      () => new Promise((resolve) => { resolveFirst = resolve; }),
    );
    searchMock.mockResolvedValueOnce({ hits: [backendHit({ id: 'msg-2', thread_id: 'thread-2' })], warnings: [] });

    const { query, results } = useSearch();
    query.value = 'alt';
    await flushDebounce();

    query.value = 'neu';
    await flushDebounce();

    expect(searchMock).toHaveBeenCalledTimes(1);
    expect(results.value).toHaveLength(0);
    resolveFirst?.({ hits: [backendHit({ id: 'msg-stale', thread_id: 'thread-stale' })], warnings: [] });
    await flushDebounce();

    expect(results.value).toHaveLength(1);
    expect(results.value[0]!.threadId).toBe('thread-2');
  });
  it('appends pages without repeated threads and ignores a page from an older query', async () => {
    const cursor = { rank_cost: 1, date: '2026-01-01', id: 'msg-1' };
    searchMock.mockResolvedValueOnce({ hits: [backendHit()], warnings: [], has_more: true, next_cursor: cursor });
    const state = useSearch();
    state.query.value = 'first';
    await flushDebounce();
    expect(state.hasMore.value).toBe(true);
    searchMock.mockResolvedValueOnce({ hits: [backendHit(), backendHit({ id: 'msg-2', thread_id: 'thread-2' })], warnings: [], has_more: true, next_cursor: { ...cursor, id: 'msg-2' } });
    await state.loadMore();
    expect(state.results.value.map((row) => row.threadId)).toEqual(['thread-1', 'thread-2']);
    expect(searchMock).toHaveBeenLastCalledWith('first', { accountId: 'acc-1', cursor });
    let finish: (value: unknown) => void = () => {};
    searchMock.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const oldPage = state.loadMore();
    state.query.value = 'second';
    await flushDebounce();
    finish({ hits: [backendHit({ id: 'stale', thread_id: 'stale' })], warnings: [], has_more: false, next_cursor: null });
    await oldPage;
    expect(state.results.value.some((row) => row.threadId === 'stale')).toBe(false);
    expect(state.results.value.map((row) => row.threadId)).toEqual(['thread-1']);
    expect(state.hasMore.value).toBe(false);
  });

});
