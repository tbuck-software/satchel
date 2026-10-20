import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendGetSenderPreference,
  backendListThreads,
  backendMarkThreadsSpam,
  backendSetThreadTags,
  backendTrashThreads,
  backendDismissMobileNotifications,
  backendUpdateThreadOrganization,
} from '@/services/backend';
import { useThreadsStore } from './threads';
import { useMailboxesStore } from './mailboxes';

vi.mock('@/services/backend', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/backend')>();
  return {
    ...actual,
    backendGetSenderPreference: vi.fn(),
    backendListThreads: vi.fn(),
    backendMarkThreadsSpam: vi.fn(),
    backendSetThreadTags: vi.fn(),
    backendTrashThreads: vi.fn(),
    backendDismissMobileNotifications: vi.fn(),
    backendUpdateThreadOrganization: vi.fn(),
  };
});

const listThreadsMock = vi.mocked(backendListThreads);
const getSenderPreferenceMock = vi.mocked(backendGetSenderPreference);
const setTagsMock = vi.mocked(backendSetThreadTags);
const updateOrganizationMock = vi.mocked(backendUpdateThreadOrganization);

function backendThread(overrides: Record<string, unknown> = {}) {
  return {
    id: 'thread-persisted',
    account_id: 'acc-work',
    subject: 'Persisted',
    normalized_subject: 'Persisted',
    snippet: 'Metadata',
    last_message_date: '2026-07-16T10:00:00Z',
    message_count: 1,
    workflow_state: 'inbox' as const,
    unread_count: 0,
    has_attachments: false,
    priority: 1,
    board_position: 12.5,
    tags: ['rechnung'],
    assignee: 'Alex',
    shared_with: ['a@example.com'],
    participants: '[{"name":"Alice","email":"alice@example.com"}]',
    mailbox_names: '["INBOX"]',
    ...overrides,
  };
}

describe('thread organization persistence', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    listThreadsMock.mockReset().mockResolvedValue({
      threads: [backendThread()],
      next_cursor: null,
      has_more: false,
    });
    getSenderPreferenceMock.mockReset();
    setTagsMock.mockReset().mockResolvedValue();
    updateOrganizationMock.mockReset().mockResolvedValue();
  });

  it('preserves the committed trash receipt when refreshing mailboxes fails', async () => {
    const receipt = { undo_token: 'receipt', undo_thread_ids: ['thread-persisted'], permanently_deleted: [] };
    vi.mocked(backendTrashThreads).mockResolvedValueOnce(receipt);
    vi.mocked(backendDismissMobileNotifications).mockResolvedValueOnce();
    const store = useThreadsStore();
    await store.load();
    listThreadsMock.mockResolvedValueOnce({ threads: [], next_cursor: null, has_more: false });
    vi.spyOn(useMailboxesStore(), 'load').mockRejectedValueOnce(new Error('mailboxes unavailable'));
    await expect(store.trashThreads(['thread-persisted'])).resolves.toEqual(receipt);
    expect(store.find('thread-persisted')).toBeUndefined();
  });

  it('hydrates persisted metadata from the backend DTO', async () => {
    const store = useThreadsStore();
    await store.load();
    expect(store.threads[0]).toMatchObject({
      id: 'thread-persisted',
      boardPosition: 12.5,
      tags: ['rechnung'],
      assignee: 'Alex',
      sharedWith: ['a@example.com'],
    });
  });

  it('normalizes sender addresses before reading their shared preference', async () => {
    getSenderPreferenceMock.mockResolvedValue({
      sender_email: 'mike@example.com',
      category: 'updates',
      is_priority: true,
    });
    const store = useThreadsStore();

    await expect(store.getSenderPreference('  Mike@Example.COM ')).resolves.toEqual({
      sender_email: 'mike@example.com',
      category: 'updates',
      is_priority: true,
    });
    expect(getSenderPreferenceMock).toHaveBeenCalledWith('mike@example.com');
  });

  it('rolls an optimistic organization change back when persistence fails', async () => {
    const store = useThreadsStore();
    await store.load();
    updateOrganizationMock.mockRejectedValueOnce(new Error('offline'));
    await expect(store.setAssignee('thread-persisted', 'Neu')).rejects.toThrow('offline');
    expect(store.threads[0]!.assignee).toBe('Alex');
  });

  // The tag editor in ThreadView is the only writer; it sends the whole list.
  it('normalizes the tags it persists for a thread', async () => {
    const store = useThreadsStore();
    await store.load();
    await store.setTagsForThread('thread-persisted', ['rechnung', ' Wichtig ', 'RECHNUNG']);
    expect(setTagsMock).toHaveBeenCalledWith(['thread-persisted'], ['rechnung', 'wichtig']);
    expect(store.threads[0]!.tags).toEqual(['rechnung', 'wichtig']);
  });

  it('serializes organization writes so a refusal restores only its own field', async () => {
    const store = useThreadsStore();
    await store.load();
    let rejectFirst!: (error: Error) => void;
    updateOrganizationMock.mockImplementationOnce(() => new Promise((_, reject) => { rejectFirst = reject; }));

    const assignee = store.setAssignee('thread-persisted', 'Neu');
    const shared = store.setSharedWith('thread-persisted', ['b@example.com']);
    await vi.waitFor(() => expect(updateOrganizationMock).toHaveBeenCalledTimes(1));
    // The second write waits: it must read the row after the first settles.
    await Promise.resolve();
    expect(updateOrganizationMock).toHaveBeenCalledTimes(1);

    rejectFirst(new Error('offline'));
    await expect(assignee).rejects.toThrow('offline');
    await shared;

    expect(updateOrganizationMock).toHaveBeenLastCalledWith({
      threadId: 'thread-persisted',
      boardPosition: 12.5,
      assignee: 'Alex',
      sharedWith: ['b@example.com'],
    });
    expect(store.threads[0]).toMatchObject({ assignee: 'Alex', sharedWith: ['b@example.com'] });
  });

  it('returns the spam receipt and drops the row before the backend answers', async () => {
    const receipt = { undo_token: 'spam', undo_thread_ids: ['thread-persisted'], permanently_deleted: [] };
    let resolveSpam!: (value: typeof receipt) => void;
    vi.mocked(backendMarkThreadsSpam).mockReturnValueOnce(new Promise((resolve) => { resolveSpam = resolve; }));
    const store = useThreadsStore();
    await store.load();
    listThreadsMock.mockResolvedValue({ threads: [], next_cursor: null, has_more: false });

    const spam = store.markThreadsSpam(['thread-persisted']);
    await vi.waitFor(() => expect(backendMarkThreadsSpam).toHaveBeenCalled());
    expect(store.find('thread-persisted')).toBeUndefined();
    resolveSpam(receipt);

    await expect(spam).resolves.toEqual(receipt);
  });

  it('does not reinsert a refused trash into a list whose scope changed meanwhile', async () => {
    let rejectTrash!: (error: Error) => void;
    vi.mocked(backendTrashThreads).mockReturnValueOnce(new Promise((_, reject) => { rejectTrash = reject; }));
    const store = useThreadsStore();
    await store.load();
    listThreadsMock.mockResolvedValue({ threads: [], next_cursor: null, has_more: false });

    const trash = store.trashThreads(['thread-persisted']);
    store.setMailboxFilter({ kind: 'all', value: null });
    rejectTrash(new Error('offline'));

    await expect(trash).rejects.toThrow('offline');
    expect(store.find('thread-persisted')).toBeUndefined();
  });

  it('puts a refused trash back in list order', async () => {
    listThreadsMock.mockResolvedValue({
      threads: [
        backendThread({ id: 'newer', last_message_date: '2026-07-17T10:00:00Z' }),
        backendThread({ id: 'older', last_message_date: '2026-07-15T10:00:00Z' }),
      ],
      next_cursor: null,
      has_more: false,
    });
    vi.mocked(backendTrashThreads).mockRejectedValueOnce(new Error('offline'));
    const store = useThreadsStore();
    await store.load();

    await expect(store.trashThreads(['newer'])).rejects.toThrow('offline');

    expect(store.threads.map((thread) => thread.id)).toEqual(['newer', 'older']);
  });

  it('rolls the tags back when the write fails', async () => {
    const store = useThreadsStore();
    await store.load();
    setTagsMock.mockRejectedValueOnce(new Error('offline'));
    await expect(store.setTagsForThread('thread-persisted', ['wichtig'])).rejects.toThrow('offline');
    expect(store.threads[0]!.tags).toEqual(['rechnung']);
  });
});

describe('thread pagination', () => {
  function page(ids: string[], nextCursor: { last_message_date: string; id: string } | null) {
    return {
      threads: ids.map((id) => backendThread({ id })),
      next_cursor: nextCursor,
      has_more: nextCursor !== null,
    };
  }

  beforeEach(() => {
    setActivePinia(createPinia());
    listThreadsMock.mockReset();
  });

  it('appends the next page instead of replacing the list', async () => {
    listThreadsMock.mockResolvedValueOnce(page(['a', 'b'], { last_message_date: 'd', id: 'b' }));
    const store = useThreadsStore();
    await store.load();
    expect(store.threads.map((t) => t.id)).toEqual(['a', 'b']);
    expect(store.hasMoreThreads).toBe(true);

    listThreadsMock.mockResolvedValueOnce(page(['c'], null));
    await store.loadMore();

    expect(store.threads.map((t) => t.id)).toEqual(['a', 'b', 'c']);
    expect(store.hasMoreThreads).toBe(false);
  });

  it('passes the cursor from the previous page', async () => {
    const cursor = { last_message_date: '2026-01-01T00:00:00Z', id: 'b' };
    listThreadsMock.mockResolvedValueOnce(page(['a', 'b'], cursor));
    const store = useThreadsStore();
    await store.load();

    listThreadsMock.mockResolvedValueOnce(page(['c'], null));
    await store.loadMore();

    expect(listThreadsMock).toHaveBeenLastCalledWith(expect.objectContaining({ cursor }));
  });

  it('does nothing when there is no further page', async () => {
    listThreadsMock.mockResolvedValueOnce(page(['a'], null));
    const store = useThreadsStore();
    await store.load();
    listThreadsMock.mockClear();

    await store.loadMore();

    expect(listThreadsMock).not.toHaveBeenCalled();
  });

  it('keeps the cursor after a failed page and pages again only on retry', async () => {
    const cursor = { last_message_date: 'd', id: 'b' };
    listThreadsMock.mockResolvedValueOnce(page(['a', 'b'], cursor));
    const store = useThreadsStore();
    await store.load();
    vi.spyOn(console, 'error').mockImplementation(() => {});

    listThreadsMock.mockRejectedValueOnce(new Error('database locked'));
    await store.loadMore();

    expect(store.loadMoreFailed).toBe(true);
    expect(store.hasMoreThreads).toBe(true);
    // Scrolling on does not hammer the failing cursor.
    listThreadsMock.mockClear();
    await store.loadMore();
    expect(listThreadsMock).not.toHaveBeenCalled();

    listThreadsMock.mockResolvedValueOnce(page(['c'], null));
    await store.retryLoadMore();

    expect(listThreadsMock).toHaveBeenLastCalledWith(expect.objectContaining({ cursor }));
    expect(store.loadMoreFailed).toBe(false);
    expect(store.threads.map((t) => t.id)).toEqual(['a', 'b', 'c']);
  });

  it('drops duplicates when a sync shifts rows between pages', async () => {
    listThreadsMock.mockResolvedValueOnce(page(['a', 'b'], { last_message_date: 'd', id: 'b' }));
    const store = useThreadsStore();
    await store.load();

    // 'b' reappears because a concurrent sync moved it.
    listThreadsMock.mockResolvedValueOnce(page(['b', 'c'], null));
    await store.loadMore();

    expect(store.threads.map((t) => t.id)).toEqual(['a', 'b', 'c']);
  });

  it('resets paging state on a fresh load', async () => {
    listThreadsMock.mockResolvedValueOnce(page(['a'], { last_message_date: 'd', id: 'a' }));
    const store = useThreadsStore();
    await store.load();
    expect(store.hasMoreThreads).toBe(true);

    listThreadsMock.mockResolvedValueOnce(page(['z'], null));
    await store.load();

    expect(store.threads.map((t) => t.id)).toEqual(['z']);
    expect(store.hasMoreThreads).toBe(false);
  });
});
