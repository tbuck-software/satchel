// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from 'vitest';
import type { FetchedServerHit } from './generated/app/commands/server_search/FetchedServerHit';
import type { BackendServerSearchHit, BackendMessageSearchResponse } from './messages';

let sendCommand: typeof import('./mock')['sendCommand'];

beforeEach(async () => {
  vi.resetModules();
  window.history.replaceState({}, '', '/?mockServerSearch=1');
  ({ sendCommand } = await import('./mock'));
}, 30_000);

it('keeps evicted-body hits openable, deduplicates cached matches and scopes provider IDs by account', async () => {
  const search = (account_id: string, query = 'needle') => sendCommand<BackendServerSearchHit[]>('server_search', { input: { account_id, query } });
  expect(await search('acc-work')).toHaveLength(2);
  expect(await search('acc-server-other')).toHaveLength(2);
  const opened = await sendCommand<FetchedServerHit>('fetch_server_search_hit', { input: { account_id: 'acc-work', remote_id: 'shared-remote' } });
  expect(opened.message_id).toBe('m-server-acc-work-true');
  expect(opened.thread_id).toBe('th-server-acc-work-true');
  const local = await sendCommand<BackendMessageSearchResponse>('search_message_hits', { input: { query: 'needle', account_id: 'acc-work' } });
  expect(local.hits.map(hit => hit.id)).toContain(opened.message_id);
  expect((await search('acc-work')).map(hit => hit.remote_id)).toEqual(['server-only']);
  expect(await search('acc-server-other')).toHaveLength(2);
  await expect(search('missing-account')).rejects.toThrow('account not found');
  expect(await search('acc-work', 'no-match')).toEqual([]);
  const downloaded = await sendCommand<FetchedServerHit>('fetch_server_search_hit', { input: { account_id: 'acc-work', remote_id: 'server-only' } });
  expect(downloaded.message_id).toBe('m-server-acc-work-false');
  expect(await search('acc-work')).toEqual([]);
});


it('can fail one provider account while the other still returns results', async () => {
  window.history.replaceState({}, '', '/?mockServerSearch=1&mockServerSearchFailure=acc-server-other');
  const responses = await Promise.allSettled(['acc-work', 'acc-server-other'].map(account_id =>
    sendCommand<BackendServerSearchHit[]>('server_search', { input: { account_id, query: 'needle' } }),
  ));
  expect(responses[0]).toMatchObject({ status: 'fulfilled', value: expect.any(Array) });
  expect(responses[1]).toMatchObject({ status: 'rejected' });
  window.history.replaceState({}, '', '/?mockServerSearch=1');
  expect(await sendCommand('server_search', { input: { account_id: 'acc-server-other', query: 'needle' } })).toHaveLength(2);
});


it('loads an evicted body through the normal body command and then keeps it cached', async () => {
  const message_id = 'm-server-acc-work-true';
  const before = await sendCommand<Array<{ body_loaded: boolean }>>('list_message_bodies', { input: { message_ids: [message_id] } });
  expect(before[0]!.body_loaded).toBe(false);
  const loaded = await sendCommand('load_message_body', { input: { message_id } });
  expect(loaded).toEqual({ body_text: 'Server search needle', body_html: null, fetched: true });
  expect(await sendCommand('load_message_body', { input: { message_id } })).toEqual({
    body_text: 'Server search needle', body_html: null, fetched: false,
  });
  const after = await sendCommand<Array<{ body_loaded: boolean }>>('list_message_bodies', { input: { message_ids: [message_id] } });
  expect(after[0]!.body_loaded).toBe(true);
});
