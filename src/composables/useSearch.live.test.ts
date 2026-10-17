import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import { backendSearchMessageHits } from '@/services/backend';
import type { MailEventHandlers } from '@/composables/useMailEvents';
import { useSearch } from './useSearch';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendSearchMessageHits: vi.fn(),
}));

/** Captures the handlers so the test can play the role of the backend. */
let captured: MailEventHandlers | null = null;
vi.mock('@/composables/useMailEvents', () => ({
  useMailEvents: (handlers: MailEventHandlers) => {
    captured = handlers;
    return () => {};
  },
}));

const search = vi.mocked(backendSearchMessageHits);

function hit(id: string) {
  return {
    id,
    thread_id: `thread-${id}`,
    subject: `Betreff ${id}`,
    snippet: '',
    from_name: 'Wispr',
    from_address: 'no-reply@wisprflow.ai',
    date: '2026-08-01T10:00:00Z',
    is_read: false,
    workflow_state: 'todo',
    folder_type: 'inbox',
  };
}

function respond(ids: string[]) {
  search.mockResolvedValue({ hits: ids.map(hit), warnings: [] } as never);
}

/** Let the search debounce elapse and every pending promise settle. */
async function settle() {
  await vi.advanceTimersByTimeAsync(300);
  await nextTick();
}

describe('useSearch live updates', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
    search.mockReset();
    captured = null;
  });

  it('parks new hits behind a count instead of resorting the visible list', async () => {
    respond(['a', 'b']);
    const { query, results, pendingCount, refresh } = useSearch({ liveUpdates: true });

    query.value = 'wispr';
    await settle();
    expect(results.value.map((r) => r.messageId)).toEqual(['a', 'b']);

    // New mail arrives and matches the query.
    respond(['c', 'a', 'b']);
    captured?.onIncoming?.();
    await settle();

    // The list the user is reading is untouched; only the badge moved.
    expect(pendingCount.value).toBe(1);
    expect(results.value.map((r) => r.messageId)).toEqual(['a', 'b']);

    refresh();
    expect(results.value.map((r) => r.messageId)).toEqual(['c', 'a', 'b']);
    expect(pendingCount.value).toBe(0);
  });

  it('reports nothing when the new mail does not match the query', async () => {
    respond(['a']);
    const { query, pendingCount } = useSearch({ liveUpdates: true });
    query.value = 'wispr';
    await settle();

    respond(['a']);
    captured?.onIncoming?.();
    await settle();

    expect(pendingCount.value).toBe(0);
  });

  it('drops a parked result set when the query changes', async () => {
    respond(['a']);
    const { query, pendingCount } = useSearch({ liveUpdates: true });
    query.value = 'wispr';
    await settle();

    respond(['b', 'a']);
    captured?.onIncoming?.();
    await settle();
    expect(pendingCount.value).toBe(1);

    // Results parked for the old query say nothing about the new one.
    respond(['z']);
    query.value = 'rechnung';
    await nextTick();
    expect(pendingCount.value).toBe(0);
  });

  it('does not subscribe at all without liveUpdates', async () => {
    respond(['a']);
    useSearch({});
    expect(captured).toBeNull();
  });
  it('resets pagination when parked results are promoted', async () => {
    const cursor = { rank_cost: 0, date: '2026-08-01T10:00:00Z', id: 'b' };
    search.mockResolvedValueOnce({ hits: [hit('a'), hit('b')], warnings: [], has_more: true, next_cursor: cursor });
    const state = useSearch({ liveUpdates: true });
    state.query.value = 'wispr';
    await settle();
    search.mockResolvedValueOnce({ hits: [hit('c')], warnings: [], has_more: false, next_cursor: null });
    await state.loadMore();
    expect(state.results.value).toHaveLength(3);
    search.mockResolvedValueOnce({ hits: [hit('new'), hit('a')], warnings: [], has_more: true, next_cursor: { ...cursor, id: 'a' } });
    captured!.onIncoming?.();
    await settle();
    expect(state.results.value).toHaveLength(3);
    state.refresh();
    expect(state.results.value.map((row) => row.messageId)).toEqual(['new', 'a']);
    expect(state.hasMore.value).toBe(true);
    search.mockResolvedValueOnce({ hits: [hit('b')], warnings: [], has_more: false, next_cursor: null });
    await state.loadMore();
    expect(search.mock.calls.at(-1)?.[1]?.cursor?.id).toBe('a');
    expect(state.results.value.map((row) => row.messageId)).toEqual(['new', 'a', 'b']);
  });

});
