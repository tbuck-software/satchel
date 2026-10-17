import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendListCachedThreads,
  backendListSmartInboxSummary,
  backendListThreads,
} from '@/services/backend';
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
const listSummary = vi.mocked(backendListSmartInboxSummary);

// A sender the front-end heuristic reads as a machine ("donotreply" without
// separators) while the SQL rules of an older backend do not.
function machineRow() {
  return {
    id: 'netcup',
    account_id: 'acc',
    subject: 'Rechnung',
    snippet: '',
    participants: JSON.stringify([{ name: 'netcup GmbH', email: 'donotreply@netcup.test' }]),
    last_message_date: '2026-08-16T10:00:00Z',
    message_count: 1,
    unread_count: 1,
    is_starred: false,
    is_muted: false,
    workflow_state: 'inbox',
    category: 'personal',
    tags: [],
    mailbox_names: JSON.stringify(['INBOX']),
    folder_type: 'inbox',
  };
}

describe('smart inbox bucket rows', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    listThreads.mockReset();
    listCachedThreads.mockReset();
    listCachedThreads.mockResolvedValue(null);
    listSummary.mockReset();
  });

  it('drops a bucket row the server summary does not know', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValue(
      { threads: [machineRow()], next_cursor: null, has_more: false } as never,
    );
    listSummary.mockResolvedValue({ total_count: 1, buckets: [] } as never);

    await threads.load();

    // Clicking such a row would query `smart_bucket=notifications`, get nothing
    // back and blank the list. The mail belongs in the plain list instead.
    expect(threads.smartInboxBuckets).toEqual([]);
    expect(
      threads.smartListEntries
        .filter((entry) => entry.kind === 'thread')
        .map((entry) => entry.thread.id),
    ).toEqual(['netcup']);
  });

  it('keeps a bucket row the server summary confirms', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValue(
      { threads: [machineRow()], next_cursor: null, has_more: false } as never,
    );
    listSummary.mockResolvedValue({
      total_count: 1,
      buckets: [
        {
          key: 'notifications',
          count: 1,
          unread_count: 1,
          preview_items: [
            {
              id: 'netcup',
              sender_name: 'netcup GmbH',
              sender_email: 'donotreply@netcup.test',
              subject: 'Rechnung',
              unread_count: 1,
            },
          ],
        },
      ],
    } as never);

    await threads.load();

    expect(
      threads.smartInboxBuckets.map((bucket) => [bucket.key, bucket.threadIds]),
    ).toEqual([['notifications', ['netcup']]]);
  });

  it('keeps backend-classified notifications out of the plain inbox after refresh', async () => {
    const threads = useThreadsStore();
    const row = {
      ...machineRow(), id: 'pipeline',
      participants: JSON.stringify([{ name: 'Build service', email: 'ci@code.example' }]),
      sender_known: true,
      smart_bucket: 'notifications',
    };
    listThreads.mockResolvedValue({ threads: [row], next_cursor: null, has_more: false } as never);
    listSummary.mockResolvedValue({ total_count: 1, buckets: [{
      key: 'notifications', count: 1, unread_count: 1, preview_items: [],
    }] } as never);
    await threads.load();
    expect(threads.smartInboxBuckets[0]?.threadIds).toEqual(['pipeline']);
    expect(threads.smartListEntries.filter((entry) => entry.kind === 'thread')).toEqual([]);
    await threads.refreshHead();
    expect(threads.smartInboxBuckets[0]?.threadIds).toEqual(['pipeline']);
    expect(threads.smartListEntries.filter((entry) => entry.kind === 'thread')).toEqual([]);
  });

  /**
   * The card's count is the server's answer, its rows come from the loaded
   * window. When the SQL classifier calls nine threads priority and the local
   * one calls none of them that, the card used to render a header, a rule and
   * "+9 more" with nothing between — a fold over an empty card. Without rows
   * to lead with, priority belongs on the folder row like every other bucket.
   */
  it('keeps priority off the card when it has no rows to show', async () => {
    const threads = useThreadsStore();
    listThreads.mockResolvedValue(
      { threads: [machineRow()], next_cursor: null, has_more: false } as never,
    );
    listSummary.mockResolvedValue({
      total_count: 10,
      buckets: [
        {
          key: 'priority',
          count: 9,
          unread_count: 9,
          preview_items: [
            {
              id: 'p-1',
              sender_name: 'Wichtig',
              sender_email: 'chef@example.test',
              subject: 'Bitte heute',
              unread_count: 1,
            },
          ],
        },
        {
          key: 'notifications',
          count: 1,
          unread_count: 1,
          preview_items: [
            {
              id: 'netcup',
              sender_name: 'netcup GmbH',
              sender_email: 'donotreply@netcup.test',
              subject: 'Rechnung',
              unread_count: 1,
            },
          ],
        },
      ],
    } as never);

    await threads.load();

    expect(threads.smartListEntries.filter((entry) => entry.kind === 'priority-card')).toEqual([]);
    const folderRow = threads.smartListEntries.find((entry) => entry.kind === 'folder-row');
    expect(folderRow?.folders.map((folder) => folder.key)).toContain('priority');
  });
});
