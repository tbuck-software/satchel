// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendThreadCursor, BackendThreadPage, BackendTrashReceipt } from './threads';

describe('mock thread pagination', () => {
  beforeEach(() => {
    vi.resetModules();
    window.history.replaceState({}, '', '/?mockExtraThreads=2400');
  });

  it('pages over more than 2000 rows in both directions using the requested scope', async () => {
    const { sendCommand } = await import('./mock');
    const scope = { account_id: 'acc-work', workflow_state: 'done', limit: 400 };
    const defaultPage = await sendCommand<BackendThreadPage>('list_threads', { input: {
      account_id: 'acc-work', workflow_state: 'done',
    } });
    expect(defaultPage.threads).toHaveLength(200);
    const cappedPage = await sendCommand<BackendThreadPage>('list_threads', { input: { ...scope, limit: 5000 } });
    expect(cappedPage.threads).toHaveLength(1000);
    const minimumPage = await sendCommand<BackendThreadPage>('list_threads', { input: { ...scope, limit: 0 } });
    expect(minimumPage.threads).toHaveLength(1);
    const pages: BackendThreadPage[] = [];
    let cursor: BackendThreadCursor | null = null;
    do {
      const page: BackendThreadPage = await sendCommand<BackendThreadPage>('list_threads', { input: { ...scope, cursor } });
      pages.push(page);
      cursor = page.next_cursor;
      expect(pages.length).toBeLessThan(10);
    } while (cursor);
    const rows = pages.flatMap(page => page.threads);
    expect(rows.length).toBeGreaterThanOrEqual(2400);
    expect(new Set(rows.map(row => row.id)).size).toBe(rows.length);
    expect(pages[0]!.has_more).toBe(true);
    expect(rows.every(row => row.account_id === 'acc-work' && row.workflow_state === 'done')).toBe(true);
    const anchor = pages[3]!.threads[0]!;
    const previous = await sendCommand<BackendThreadPage>('list_threads', { input: {
      ...scope, before_cursor: { id: anchor.id, last_message_date: anchor.last_message_date },
    } });
    expect(previous.threads.map(row => row.id)).toEqual(pages[2]!.threads.map(row => row.id));
    const inbox = await sendCommand<BackendThreadPage>('list_threads', { input: {
      mailbox_kind: 'section', mailbox_value: 'inbox', limit: 1000,
    } });
    expect(inbox.threads.every(row => ['inbox', 'todo'].includes(row.workflow_state))).toBe(true);
    expect(inbox.threads.some(row => row.id.startsWith('th-extra-'))).toBe(false);
    const other = await sendCommand<BackendThreadPage>('list_threads', { input: {
      account_id: 'does-not-exist', limit: 2,
    } });
    expect(other).toEqual({ threads: [], next_cursor: null, has_more: false });
  }, 20000);
});

describe('mock spam receipt', () => {
  beforeEach(() => {
    vi.resetModules();
    window.history.replaceState({}, '', '/');
  });

  it('answers like the backend: a receipt that undo_trash reverses', async () => {
    const { sendCommand } = await import('./mock');
    const listed = await sendCommand<BackendThreadPage>('list_threads', { input: { limit: 1 } });
    const id = listed.threads[0]!.id;

    const receipt = await sendCommand<BackendTrashReceipt>('mark_threads_spam', { input: { thread_ids: [id] } });
    expect(receipt.undo_thread_ids).toEqual([id]);
    expect(receipt.undo_token).toBeTruthy();
    await expect(sendCommand('get_thread', { threadId: id })).rejects.toBeTruthy();

    await sendCommand('undo_trash', { receiptId: receipt.undo_token });
    await expect(sendCommand<{ id: string }>('get_thread', { threadId: id })).resolves.toMatchObject({ id });
  });
});
