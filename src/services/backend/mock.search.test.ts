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
});
