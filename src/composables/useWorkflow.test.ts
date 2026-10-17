import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useThreadsStore } from '@/stores/threads';
import type { Thread } from '@/types/mail';
import { THREAD_LEAVE_MS, useWorkflow } from './useWorkflow';

vi.mock('@/services/backend', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend')>(),
  backendCommandsAvailable: () => true,
  backendTransitionThreads: vi.fn(async (input: { threads: Array<{ thread_id: string }> }) => ({
    applied: input.threads.map(thread => thread.thread_id), failed: [],
  })),
  backendSetThreadsRead: vi.fn(async () => {}),
  backendDismissMobileNotifications: vi.fn(async () => {}),
  backendListThreads: vi.fn(async () => ({ threads: [], next_cursor: null, has_more: false })),
  backendListCachedThreads: vi.fn(async () => null),
  backendListSmartInboxSummary: vi.fn(async () => null),
  backendListMailboxes: vi.fn(async () => []),
}));

function row(): Thread {
  return {
    id: 'th-leave', accountId: 'acc-work', subject: 'Leaving row',
    participants: [{ name: 'Sender', email: 'sender@example.com' }],
    snippet: '', lastMessageAt: '2026-09-04T10:00:00Z', lastMessageAtMs: Date.parse('2026-09-04T10:00:00Z'), messageCount: 1,
    unreadCount: 0, hasAttachments: false, workflowState: 'inbox', priority: 1,
    mailboxNames: ['INBOX'], tags: [],
  };
}

describe('useWorkflow leave animation', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    useThreadsStore().threads = [row()];
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('keeps the row while it animates out and flips the state afterwards', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    const target = threads.threads[0];
    target!.workflowState = 'inbox';

    const pending = workflow.transitionMany([target!.id], 'done');

    // The optimistic flip is deferred so the row can animate out first —
    // removing it in the same frame as the click reads as a glitch.
    expect(threads.leavingThreadIds).toContain(target!.id);
    expect(target!.workflowState).toBe('inbox');

    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);

    expect(threads.find(target!.id)?.workflowState).toBe('done');
    expect(threads.leavingThreadIds).not.toContain(target!.id);
    await pending;
  });

  it('ignores a second check-off while the row is still leaving', async () => {
    const threads = useThreadsStore();
    const workflow = useWorkflow();
    const target = threads.threads[0];
    target!.workflowState = 'inbox';

    const first = workflow.transitionMany([target!.id], 'done');

    // Without the guard the second call would queue a second archive.
    await expect(workflow.transitionMany([target!.id], 'done')).resolves.toEqual([]);

    await vi.advanceTimersByTimeAsync(THREAD_LEAVE_MS);
    await expect(first).resolves.toEqual([target!.id]);
    expect(threads.find(target!.id)?.workflowState).toBe('done');
  });
});
