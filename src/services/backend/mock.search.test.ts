// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendMessageSearchResponse } from './messages';

describe('mock message search pagination', () => {
  beforeEach(() => {
    vi.resetModules();
    window.history.replaceState({}, '', '/?mockExtraThreads=137');
  });

  it('honors account scope, operators and page size over more than fifty threads', async () => {
    const { sendCommand } = await import('./mock');
    const pages: BackendMessageSearchResponse[] = [];
    let cursor: BackendMessageSearchResponse['next_cursor'] = null;
    do {
      const page: BackendMessageSearchResponse = await sendCommand('search_message_hits', { input: {
        query: 'in:done', account_id: 'acc-work', limit: 31, cursor,
      } });
      expect(page.hits.length).toBeLessThanOrEqual(31);
      pages.push(page);
      cursor = page.next_cursor;
    } while (cursor);
    const hits = pages.flatMap((page) => page.hits);
    expect(hits.length).toBeGreaterThanOrEqual(137);
    expect(new Set(hits.map((hit) => hit.thread_id)).size).toBe(hits.length);
    expect(hits.every((hit) => hit.workflow_state === 'done')).toBe(true);
    expect(pages[0]!.has_more).toBe(true);
    expect(pages.at(-1)!.has_more).toBe(false);
    const other: BackendMessageSearchResponse = await sendCommand('search_message_hits', { input: {
      query: 'in:done', account_id: 'missing-account', limit: 31, cursor: null,
    } });
    expect(other.hits).toEqual([]);
    const inbox: BackendMessageSearchResponse = await sendCommand('search_message_hits', { input: {
      query: 'in:inbox', limit: 500, cursor: null,
    } });
    expect(inbox.hits.every((hit) => hit.workflow_state === 'inbox')).toBe(true);
    expect(inbox.hits.some((hit) => hit.thread_id.startsWith('th-extra-'))).toBe(false);
    const empty: BackendMessageSearchResponse = await sendCommand('search_message_hits', { input: { query: '', limit: 8, cursor: null } });
    expect(empty.hits).toEqual([]);
    const ignored: BackendMessageSearchResponse = await sendCommand('search_message_hits', { input: { query: 'in:unknown', cursor: null } });
    expect(ignored.hits).toEqual([]);
    expect(ignored.warnings).toEqual([{ operator: 'in', value: 'unknown' }]);
  });

  it('narrows to a list exactly as list_threads draws it', async () => {
    const { sendCommand } = await import('./mock');
    const search = async (list_scope: Record<string, string | undefined> | null) => {
      const page: BackendMessageSearchResponse = await sendCommand('search_message_hits', { input: {
        query: 'e', limit: 500, cursor: null, list_scope,
      } });
      return page.hits;
    };
    // The fixture has no to-do mail in the Inbox; make one.
    const [todo] = await search({ mailbox_kind: 'section', mailbox_value: 'inbox' });
    await sendCommand('transition_threads', { input: {
      threads: [{ thread_id: todo!.thread_id, from: 'inbox' }], to: 'todo',
    } });
    // Nor any mail waiting for a reply, which Later shows next to snoozed mail.
    const [waiting] = await search({ workflow_state: 'done' });
    await sendCommand('transition_threads', { input: {
      threads: [{ thread_id: waiting!.thread_id, from: 'done' }], to: 'waiting',
    } });
    for (const scope of [
      { mailbox_kind: 'section', mailbox_value: 'inbox' },
      { workflow_state: 'done' },
      { workflow_state: 'later' },
      { mailbox_kind: 'folder', mailbox_value: 'Archive' },
    ]) {
      const listed: { threads: Array<{ id: string; workflow_state: string }> } = await sendCommand('list_threads', {
        input: { ...scope, limit: 1000 },
      });
      const inList = new Set(listed.threads.map((thread) => thread.id));
      const everywhere = await search(null);
      const scoped = await search(scope);
      expect(scoped.map((hit) => hit.thread_id)).toEqual(
        everywhere.filter((hit) => inList.has(hit.thread_id)).map((hit) => hit.thread_id),
      );
      expect(scoped.length, JSON.stringify(scope)).toBeGreaterThan(0);
    }
    // The Inbox is inbox *and* todo — what `in:inbox` alone does not say.
    const inbox = await search({ mailbox_kind: 'section', mailbox_value: 'inbox' });
    expect(new Set(inbox.map((hit) => hit.workflow_state))).toEqual(new Set(['inbox', 'todo']));
    // Later is snoozed *and* waiting — no single state says both.
    const later = await search({ workflow_state: 'later' });
    expect(new Set(later.map((hit) => hit.workflow_state))).toEqual(new Set(['snoozed', 'waiting']));
  });
});
