// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  BackendNotSpamReceipt,
  BackendSpamFilterStatus,
  BackendThread,
  BackendThreadPage,
  BackendTrashReceipt,
} from '@/services/backend';

beforeEach(() => {
  vi.resetModules();
  window.history.replaceState({}, '', '/');
});

async function firstThreadId(sendCommand: <T>(command: string, payload?: Record<string, unknown>) => Promise<T>) {
  const listed = await sendCommand<BackendThreadPage>('list_threads', { input: { limit: 1 } });
  return listed.threads[0]!.id;
}

describe('mock spam filter', () => {
  it('reports the filter like the backend: Mark by default, not ready untrained', async () => {
    const { sendCommand } = await import('./mock');
    const status = await sendCommand<BackendSpamFilterStatus>('get_spam_filter_status');
    expect(status).toMatchObject({
      mode: 'mark', spam_messages: 0, ham_messages: 0, ready: false, bootstrap_pending: false,
      spam_needed: 50, ham_needed: 50, move_ready: false, move_spam_needed: 200, move_ham_needed: 200,
    });

    await sendCommand('set_spam_filter_mode', { input: { mode: 'move' } });
    expect((await sendCommand<BackendSpamFilterStatus>('get_spam_filter_status')).mode).toBe('move');
    await expect(sendCommand('set_spam_filter_mode', { input: { mode: 'everything' } })).rejects.toBeTruthy();
  });

  it('learns from Spam and unlearns on its Undo', async () => {
    const { sendCommand } = await import('./mock');
    const id = await firstThreadId(sendCommand);
    const receipt = await sendCommand<BackendTrashReceipt>('mark_threads_spam', { input: { thread_ids: [id] } });
    expect((await sendCommand<BackendSpamFilterStatus>('get_spam_filter_status')).spam_messages).toBe(1);

    await sendCommand('undo_trash', { receiptId: receipt.undo_token });
    expect((await sendCommand<BackendSpamFilterStatus>('get_spam_filter_status')).spam_messages).toBe(0);
  });

  it('Not spam clears the mark, learns ham and moves Spam-folder mail to the Inbox', async () => {
    // The mock copies its fixtures when it loads: seed them first.
    const { mockThreads: threads } = await import('@/data/mock');
    const marked = threads[0]!;
    const filed = threads[1]!;
    marked.likelySpam = true;
    filed.mailboxNames = ['Spam'];
    filed.workflowState = 'done';
    const { sendCommand } = await import('./mock');

    const receipt = await sendCommand<BackendNotSpamReceipt>('mark_threads_not_spam', {
      input: { thread_ids: [marked.id, filed.id, marked.id] },
    });
    expect(receipt).toMatchObject({ thread_ids: [marked.id, filed.id], moved_thread_ids: [filed.id] });
    expect(receipt.undo_token).toBeTruthy();

    const markedDto = await sendCommand<BackendThread>('get_thread', { threadId: marked.id });
    expect(markedDto.likely_spam).toBe(false);
    const filedDto = await sendCommand<BackendThread>('get_thread', { threadId: filed.id });
    expect(JSON.parse(filedDto.mailbox_names)).toEqual(['INBOX']);
    expect(filedDto.workflow_state).toBe('inbox');
    expect((await sendCommand<BackendSpamFilterStatus>('get_spam_filter_status')).ham_messages).toBe(2);

    // Undo unlearns it and puts the mark and the Spam folder back.
    await sendCommand('undo_not_spam', { input: { receipt } });
    expect((await sendCommand<BackendThread>('get_thread', { threadId: marked.id })).likely_spam).toBe(true);
    const back = await sendCommand<BackendThread>('get_thread', { threadId: filed.id });
    expect(JSON.parse(back.mailbox_names)).toEqual(['Spam']);
    expect(back.workflow_state).toBe('done');
    expect((await sendCommand<BackendSpamFilterStatus>('get_spam_filter_status')).ham_messages).toBe(0);
  });
});
