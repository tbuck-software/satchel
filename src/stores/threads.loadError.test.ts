import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { backendListCachedThreads, backendListThreads } from '@/services/backend';
import { useConnectionStore } from './connection';
import { useThreadsStore } from './threads';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendListCachedThreads: vi.fn(),
  backendListThreads: vi.fn(),
  backendListSmartInboxSummary: vi.fn(),
  backendGetThread: vi.fn(),
  backendMarkThreadsSpam: vi.fn(),
  backendSetThreadCategory: vi.fn(),
  backendSetThreadMuted: vi.fn(),
  backendSetThreadsRead: vi.fn(),
  backendSetThreadsStarred: vi.fn(),
  backendSetThreadTags: vi.fn(),
  backendTrashThreads: vi.fn(),
  backendUpdateThreadState: vi.fn(),
  backendUpdateThreadOrganization: vi.fn(),
}));

const listThreads = vi.mocked(backendListThreads);
const listCachedThreads = vi.mocked(backendListCachedThreads);

function page() {
  return {
    threads: [{
      id: 'thread-1',
      account_id: 'acc',
      subject: 'Subject',
      normalized_subject: 'Subject',
      snippet: '',
      participants: '[]',
      last_message_date: '2026-09-06T12:00:00Z',
      message_count: 1,
      unread_count: 0,
      has_attachments: false,
      priority: 1 as const,
      is_starred: false,
      is_muted: false,
      workflow_state: 'inbox' as const,
      tags: [],
      mailbox_names: JSON.stringify(['INBOX']),
      folder_type: 'inbox',
    }],
    next_cursor: null,
    has_more: false,
  };
}

describe('threads.load failures', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    listThreads.mockReset();
    listCachedThreads.mockReset().mockResolvedValue(null);
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
  });

  it('keeps the failure readable per subsystem instead of leaving an empty list', async () => {
    listThreads.mockRejectedValueOnce(new Error('db is locked'));
    const threads = useThreadsStore();
    const connection = useConnectionStore();

    await threads.load();

    expect(threads.threads).toHaveLength(0);
    expect(connection.backendErrorFor('threads')).toBe('db is locked');
    expect(connection.backendErrorFor('messages')).toBeNull();
  });

  it('drops the failure once the same load path succeeds', async () => {
    listThreads.mockRejectedValueOnce(new Error('db is locked'));
    const threads = useThreadsStore();
    const connection = useConnectionStore();
    await threads.load();

    listThreads.mockResolvedValueOnce(page());
    await threads.load();

    expect(threads.threads).toHaveLength(1);
    expect(connection.backendErrorFor('threads')).toBeNull();
  });
});
