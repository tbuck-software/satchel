import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendDismissMobileNotifications,
  backendListMailboxes,
  backendListSmartInboxSummary,
  backendListThreads,
  backendSetThreadsRead,
} from '@/services/backend';
import type { Thread } from '@/types/mail';
import { useMailboxesStore } from './mailboxes';
import { useThreadsStore } from './threads';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendListCachedThreads: vi.fn(async () => null),
  backendListThreads: vi.fn(async () => ({ threads: [], next_cursor: null, has_more: false })),
  backendListSmartInboxSummary: vi.fn(async () => null),
  backendGetThread: vi.fn(),
  backendGetThreads: vi.fn(async () => []),
  backendListMailboxes: vi.fn(async () => []),
  backendSetThreadsRead: vi.fn(async () => {}),
  backendDismissMobileNotifications: vi.fn(async () => {}),
}));

function thread(id: string, overrides: Partial<Thread> = {}): Thread {
  return {
    id,
    accountId: 'acc',
    subject: id,
    participants: [],
    snippet: '',
    lastMessageAt: '2026-10-01T10:00:00Z',
    lastMessageAtMs: Date.parse('2026-10-01T10:00:00Z'),
    messageCount: 2,
    unreadCount: 2,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: [],
    ...overrides,
  };
}

function mailbox(id: string, name: string, type: string, unread: number) {
  return { id, account_id: 'acc', account_email: 'me@example.com', name, mailbox_type: type, thread_count: 10, unread_count: unread };
}

describe('marking threads read', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('updates rows and badges at once and reconciles once the reader pauses', async () => {
    const threads = useThreadsStore();
    const mailboxes = useMailboxesStore();
    threads.threads = [
      thread('t1'),
      thread('t2', { mailboxNames: ['INBOX', 'Projects'] }),
      thread('t3', { workflowState: 'done' }),
    ];
    mailboxes.mailboxes = [mailbox('f-inbox', 'INBOX', 'inbox', 2), mailbox('f-projects', 'Projects', 'custom', 1)];

    for (const id of ['t1', 't2', 't3']) {
      await threads.setThreadsRead([id], true);
      await vi.advanceTimersByTimeAsync(900);
    }

    expect(threads.threads.map((item) => item.unreadCount)).toEqual([0, 0, 0]);
    // A done thread does not count in the Inbox badge, so it does not move it.
    expect(mailboxes.mailboxes.map((item) => item.unread_count)).toEqual([0, 0]);
    expect(backendSetThreadsRead).toHaveBeenCalledTimes(3);
    expect(backendListThreads).not.toHaveBeenCalled();
    expect(backendListMailboxes).not.toHaveBeenCalled();
    expect(backendDismissMobileNotifications).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1_500);
    expect(backendListThreads).toHaveBeenCalledTimes(1);
    expect(backendListMailboxes).toHaveBeenCalledTimes(1);
    expect(backendDismissMobileNotifications).toHaveBeenCalledTimes(1);
    expect(backendDismissMobileNotifications).toHaveBeenCalledWith(['t1', 't2', 't3']);
    expect(backendListSmartInboxSummary).toHaveBeenCalledTimes(1);
  });

  it('puts rows and badges back when the write fails', async () => {
    const threads = useThreadsStore();
    const mailboxes = useMailboxesStore();
    threads.threads = [thread('t1')];
    mailboxes.mailboxes = [mailbox('f-inbox', 'INBOX', 'inbox', 4)];
    vi.mocked(backendSetThreadsRead).mockRejectedValueOnce(new Error('offline'));

    await expect(threads.setThreadsRead(['t1'], true)).rejects.toThrow('offline');

    expect(threads.threads[0]?.unreadCount).toBe(2);
    expect(mailboxes.mailboxes[0]?.unread_count).toBe(4);
  });
});
