// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendListSenderSweepTargets,
  backendTransitionThreads,
  backendTrashThreads,
  backendUndoTrash,
} from '@/services/backend';
import type { Thread } from '@/types/mail';
import { useThreadsStore } from '@/stores/threads';
import { useToast } from '@/composables/useToast';
import { useThreadActions } from '@/composables/useThreadActions';
import { SENDER_SWEEP_BATCH } from '@/composables/senderSweepActions';

vi.mock('vue-router', () => ({ useRoute: () => undefined, useRouter: () => undefined }));

const { calls, targetCount, unreadIds } = vi.hoisted(() => ({
  calls: [] as Array<{ command: string; size: number }>,
  targetCount: { value: 0 },
  unreadIds: new Set<string>(),
}));

vi.mock('@/services/backend', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/backend')>();
  return {
    ...actual,
    backendCommandsAvailable: () => true,
    backendSupportsNativeCommands: vi.fn(() => true),
    backendListSenderSweepTargets: vi.fn(async (input: { keep_latest: boolean }) => {
      calls.push({ command: 'list_sender_sweep_targets', size: 0 });
      const ids = Array.from({ length: targetCount.value }, (_, index) => `s-${index}`)
        .slice(input.keep_latest ? 1 : 0);
      return ids.map((id) => ({
        thread_id: id,
        workflow_state: 'inbox',
        last_message_date: '2026-08-01T10:00:00+00:00',
        unread_count: unreadIds.has(id) ? 1 : 0,
      }));
    }),
    backendTransitionThreads: vi.fn(async (input: { threads: Array<{ thread_id: string }> }) => {
      calls.push({ command: 'transition_threads', size: input.threads.length });
      return { applied: input.threads.map((entry) => entry.thread_id), failed: [] };
    }),
    backendTrashThreads: vi.fn(async (threadIds: string[]) => {
      calls.push({ command: 'trash_threads', size: threadIds.length });
      return { undo_token: `receipt-${calls.length}`, undo_thread_ids: threadIds, permanently_deleted: [] };
    }),
    backendUndoTrash: vi.fn(async () => {
      calls.push({ command: 'undo_trash', size: 0 });
    }),
    backendSetThreadsRead: vi.fn(async (threadIds: string[]) => {
      calls.push({ command: 'set_threads_read', size: threadIds.length });
    }),
    backendDismissMobileNotifications: vi.fn(async () => {}),
    backendListThreads: vi.fn(async () => ({ threads: [], next_cursor: null, has_more: false })),
    backendListCachedThreads: vi.fn(async () => null),
    backendListMailboxes: vi.fn(async () => []),
    backendListSmartInboxSummary: vi.fn(async () => null),
    backendRecordFrontendDiagnostic: vi.fn(async () => {}),
  };
});

function loadedThread(id: string): Thread {
  return {
    id,
    accountId: 'acc-1',
    subject: id,
    participants: [{ name: 'News', email: 'news@shop.example' }],
    snippet: '',
    lastMessageAt: '2026-08-24T15:08:00.000Z',
    lastMessageAtMs: Date.parse('2026-08-24T15:08:00.000Z'),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: [],
  };
}

function sizes(command: string): number[] {
  return calls.filter((call) => call.command === command).map((call) => call.size);
}

describe('sender sweep', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    calls.length = 0;
    targetCount.value = 0;
    unreadIds.clear();
    useToast().clear();
  });

  afterEach(() => useToast().clear());

  it('archives thousands of threads in bounded batches with one receipt', async () => {
    targetCount.value = 2 * SENDER_SWEEP_BATCH + 50;
    const threads = useThreadsStore();
    // Only the first rows are loaded; the sweep reaches the rest by id.
    threads.threads = [loadedThread('s-0'), loadedThread('s-1')];

    const filed = await useThreadActions().sweepSender({
      accountId: 'acc-1', email: 'news@shop.example', action: 'archive',
    });

    expect(filed).toBe(targetCount.value);
    expect(sizes('transition_threads')).toEqual([SENDER_SWEEP_BATCH, SENDER_SWEEP_BATCH, 50]);
    expect(threads.find('s-0')?.workflowState).toBe('done');
    const toasts = useToast().state.toasts.filter((toast) => toast.tone === 'success');
    expect(toasts).toHaveLength(1);
    expect(toasts[0]).toMatchObject({ count: targetCount.value, groupKey: 'sweep:done' });
  });

  it('says "X of Y" when part of a sweep was refused', async () => {
    targetCount.value = 5;
    vi.mocked(backendTransitionThreads).mockImplementationOnce(async (input) => ({
      applied: input.threads.slice(1).map((entry) => entry.thread_id),
      failed: [{ thread_id: input.threads[0]!.thread_id, error: 'database is locked' }],
    }));
    await useThreadActions().sweepSender({ accountId: 'acc-1', email: 'news@shop.example', action: 'archive' });
    const receipt = useToast().state.toasts.find((toast) => toast.tone === 'success')!;
    expect(receipt.message).toBe('4 of 5 conversations done');
  });

  it('keeps the latest thread when asked', async () => {
    targetCount.value = 4;
    await useThreadActions().sweepSender({
      accountId: 'acc-1', email: 'news@shop.example', action: 'archive', keepLatest: true,
    });
    const input = vi.mocked(backendListSenderSweepTargets).mock.calls[0]![0];
    expect(input).toMatchObject({ account_id: 'acc-1', sender_email: 'news@shop.example', keep_latest: true });
    const filed = vi.mocked(backendTransitionThreads).mock.calls[0]![0].threads.map((entry) => entry.thread_id);
    expect(filed).toEqual(['s-1', 's-2', 's-3']);
  });

  it('undoes a sweep without loading each restored thread, and gives unread back', async () => {
    targetCount.value = 3;
    unreadIds.add('s-2');
    await useThreadActions().sweepSender({ accountId: 'acc-1', email: 'news@shop.example', action: 'archive' });
    calls.length = 0;

    const undo = useToast().state.toasts.find((toast) => toast.tone === 'success')!.action!;
    await undo.handler();

    const restore = vi.mocked(backendTransitionThreads).mock.calls.at(-1)![0];
    expect(restore).toMatchObject({ to: 'inbox' });
    expect(restore.threads.map((entry) => entry.thread_id)).toEqual(['s-0', 's-1', 's-2']);
    expect(sizes('set_threads_read')).toEqual([1]);
    // No per-thread `get_threads` to put the cursor back on a row far below.
    expect(calls.every((call) => ['transition_threads', 'set_threads_read'].includes(call.command))).toBe(true);
  });

  it('deletes in batches and undoes every receipt', async () => {
    targetCount.value = SENDER_SWEEP_BATCH + 1;
    await useThreadActions().sweepSender({ accountId: 'acc-1', email: 'news@shop.example', action: 'trash' });

    expect(sizes('trash_threads')).toEqual([SENDER_SWEEP_BATCH, 1]);
    const toast = useToast().state.toasts.find((entry) => entry.tone === 'success')!;
    expect(toast.message).toContain(String(SENDER_SWEEP_BATCH + 1));
    await toast.action!.handler();
    expect(vi.mocked(backendUndoTrash).mock.calls.map(([token]) => token)).toHaveLength(2);
  });

  it('says so when there is nothing to sweep', async () => {
    targetCount.value = 0;
    const filed = await useThreadActions().sweepSender({
      accountId: 'acc-1', email: 'news@shop.example', action: 'trash',
    });
    expect(filed).toBe(0);
    expect(backendTrashThreads).not.toHaveBeenCalled();
    expect(useToast().state.toasts[0]?.tone).toBe('info');
  });
});
