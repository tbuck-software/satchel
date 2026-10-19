import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useThreadsStore } from './threads';
import { useSettingsStore } from './settings';
import * as backend from '@/services/backend';

vi.mock('@/services/backend', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend')>(),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendSetThreadsStarred: vi.fn(),
  backendSetThreadMuted: vi.fn(),
  backendSetThreadCategory: vi.fn(),
  backendSetThreadsRead: vi.fn(),
  backendMarkThreadsSpam: vi.fn(),
  backendSetSenderPreference: vi.fn(),
}));

beforeEach(() => {
  setActivePinia(createPinia());
  vi.mocked(backend.backendSupportsNativeCommands).mockReturnValue(true);
  vi.mocked(backend.backendCommandsAvailable).mockReturnValue(true);
  for (const method of [backend.backendSetThreadsStarred, backend.backendSetThreadMuted,
    backend.backendSetThreadCategory, backend.backendSetThreadsRead, backend.backendMarkThreadsSpam,
    backend.backendSetSenderPreference]) {
    vi.mocked(method).mockReset().mockRejectedValue(new Error('database is locked'));
  }
  useThreadsStore().threads = [{
    id: 'mail', accountId: 'acc-work', subject: 'Test', participants: [{ email: 'sender@example.com', name: 'Sender' }],
    snippet: '', lastMessageAt: '2026-09-04', lastMessageAtMs: Date.parse('2026-09-04'), messageCount: 3, unreadCount: 3,
    hasAttachments: false, workflowState: 'inbox', priority: 0, isStarred: false,
    isMuted: false, category: 'personal', hasInvite: false, mailboxNames: ['INBOX'], tags: [],
  }];
});

describe('thread action failures', () => {
  it.each([
    ['starring', (store: ReturnType<typeof useThreadsStore>) => store.setThreadsStarred(['mail'], true)],
    ['muting', (store: ReturnType<typeof useThreadsStore>) => store.setThreadMuted('mail', true)],
    ['category', (store: ReturnType<typeof useThreadsStore>) => store.setThreadCategory('mail', 'updates')],
    ['read', (store: ReturnType<typeof useThreadsStore>) => store.setThreadsRead(['mail'], true)],
    ['spam', (store: ReturnType<typeof useThreadsStore>) => store.markThreadsSpam(['mail'])],
  ])('rolls back %s and rejects instead of reporting success', async (_, change) => {
    const store = useThreadsStore();
    const before = structuredClone(JSON.parse(JSON.stringify(store.threads)));
    await expect(change(store)).rejects.toThrow('database is locked');
    expect(store.threads).toEqual(before);
  });

  it('serializes overlapping failures without restoring an uncommitted value', async () => {
    const store = useThreadsStore();
    const results = await Promise.allSettled([
      store.setThreadsStarred(['mail'], true), store.setThreadsStarred(['mail'], false),
    ]);
    expect(results.map((r) => r.status)).toEqual(['rejected', 'rejected']);
    expect(store.threads[0]?.isStarred).toBe(false);
  });

  it('stars a batch in one round trip and rolls all of it back on refusal', async () => {
    const store = useThreadsStore();
    store.threads = [...store.threads, { ...store.threads[0]!, id: 'mail-2' }];

    await expect(store.setThreadsStarred(['mail', ' mail-2 ', 'mail'], true))
      .rejects.toThrow('database is locked');

    // The command flags the whole selection in one transaction, so a refusal
    // means none of the rows changed.
    expect(backend.backendSetThreadsStarred).toHaveBeenCalledTimes(1);
    expect(backend.backendSetThreadsStarred).toHaveBeenCalledWith(['mail', 'mail-2'], true);
    expect(store.threads.map((thread) => thread.isStarred)).toEqual([false, false]);
  });

  it('keeps an explicit browser fallback local', async () => {
    vi.mocked(backend.backendCommandsAvailable).mockReturnValue(false);
    const store = useThreadsStore();
    await store.setThreadsStarred(['mail'], true);
    expect(store.threads[0]?.isStarred).toBe(true);
  });

  it('does not save a sender bucket when its database write fails', async () => {
    const store = useThreadsStore();
    await expect(store.setSenderCategoryByEmail('acc-work', 'sender@example.com', 'updates', 'notifications'))
      .rejects.toThrow('database is locked');
    expect(useSettingsStore().inboxSenderBuckets['sender@example.com']).toBeUndefined();
    expect(store.threads[0]?.category).toBe('personal');
  });


});
