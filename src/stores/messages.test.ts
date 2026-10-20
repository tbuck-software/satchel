// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendCommandsAvailable,
  backendDownloadAttachment,
  backendGetMessageSource,
  backendListAttachmentsForMessages,
  backendListMessageBodies,
  backendListMessagesPreview,
  backendLoadMessageBody,
  backendLoadThreadMessages,
  backendSupportsNativeCommands,
  backendUnsubscribe,
} from '@/services/backend';
import { useAccountsStore } from './accounts';
import { useConnectionStore } from './connection';
import { bodyHtmlWithInlineImages, THREAD_RETENTION_IDLE_MS, useMessagesStore } from './messages';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => false),
  backendSupportsNativeCommands: vi.fn(() => false),
  backendDownloadAttachment: vi.fn(),
  backendGetMessageSource: vi.fn(),
  backendListAttachmentsForMessages: vi.fn(async () => []),
  backendListMessageBodies: vi.fn(async () => []),
  backendListMessagesPreview: vi.fn(),
  backendLoadMessageBody: vi.fn(),
  backendUnsubscribe: vi.fn(),
  // The backend's `load_thread_messages`, built from the two calls it joins.
  backendLoadThreadMessages: vi.fn(async (threadId: string, bodyCount: number) => {
    const backend = await import('@/services/backend');
    const messages = await backend.backendListMessagesPreview(threadId);
    const conversation = messages.filter((message) => !message.is_draft).slice(-bodyCount);
    const ids = [...conversation, ...messages.filter((message) => message.is_draft)]
      .map((message) => message.id);
    const bodies = ids.length > 0 ? await backend.backendListMessageBodies(ids) : [];
    return { messages, bodies };
  }),
}));

vi.mock('@/utils/diagnostics', () => ({
  recordFrontendDiagnostic: vi.fn(),
}));

const previewMock = vi.mocked(backendListMessagesPreview);
const bodiesMock = vi.mocked(backendListMessageBodies);
const attachmentsMock = vi.mocked(backendListAttachmentsForMessages);
const downloadAttachmentMock = vi.mocked(backendDownloadAttachment);
const loadBodyMock = vi.mocked(backendLoadMessageBody);
const nativeCommandsMock = vi.mocked(backendSupportsNativeCommands);
const commandsAvailableMock = vi.mocked(backendCommandsAvailable);
const messageSourceMock = vi.mocked(backendGetMessageSource);
const unsubscribeMock = vi.mocked(backendUnsubscribe);
const openMock = vi.mocked(backendLoadThreadMessages);

function backendMessage(overrides: Record<string, unknown> = {}) {
  return {
    id: 'm-live',
    thread_id: 'th-live',
    subject: 'Live message',
    snippet: 'Preview only',
    from_address: 'sender@example.com',
    from_name: 'Sender',
    to_addresses: ['you@example.com'],
    cc_addresses: [],
    date: '2026-07-11T10:00:00Z',
    body_text: 'Preview only',
    body_html: null,
    body_loaded: false,
    is_read: true,
    is_draft: false,
    pgp_state: null,
    pgp_error_detail: null,
    pgp_signer_name: null,
    pgp_signer_fingerprint: null,
    smime_state: null,
    smime_signer_email: null,
    smime_signer_cn: null,
    smime_error_detail: null,
    list_unsubscribe: null,
    list_unsubscribe_post: null,
    ...overrides,
  };
}

function inlineBody(html: string) {
  return {
    id: 'm-live',
    snippet: 'Hydrated',
    body_text: 'Hydrated body',
    body_html: html,
    body_loaded: true,
    pgp_state: null,
    pgp_error_detail: null,
    pgp_signer_name: null,
    pgp_signer_fingerprint: null,
    smime_state: null,
    smime_signer_email: null,
    smime_signer_cn: null,
    smime_error_detail: null,
  };
}

function inlineAttachmentRow(overrides: Record<string, unknown> = {}) {
  return {
    id: 'att-inline',
    message_id: 'm-live',
    filename: 'image001.jpg',
    content_type: 'image/jpeg',
    size: 3574,
    content_id: 'image001.jpg@01DD16D4.C2220470',
    is_inline: 1,
    cached_path: null,
    ...overrides,
  };
}

describe('messages store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    nativeCommandsMock.mockReturnValue(false);
    commandsAvailableMock.mockReturnValue(false);
    previewMock.mockReset();
    bodiesMock.mockReset();
    bodiesMock.mockResolvedValue([]);
    attachmentsMock.mockReset();
    attachmentsMock.mockResolvedValue([]);
    downloadAttachmentMock.mockReset();
    openMock.mockClear();
    messageSourceMock.mockReset();
    unsubscribeMock.mockReset();
  });

  it('does not request message source data outside the native backend', async () => {
    const messages = useMessagesStore();

    await expect(messages.loadMessageSource('m-web')).resolves.toBeNull();
    expect(messageSourceMock).not.toHaveBeenCalled();
  });

  it('loads message source data through the native backend seam', async () => {
    commandsAvailableMock.mockReturnValue(true);
    const source = {
      message_id: 'm-desktop',
      subject: 'Original subject',
      raw_headers: 'From: sender@example.com',
      body_text: 'Original body',
      body_html: null,
      raw_size: 42,
    };
    messageSourceMock.mockResolvedValue(source);
    const messages = useMessagesStore();

    await expect(messages.loadMessageSource('m-desktop')).resolves.toEqual(source);
    expect(messageSourceMock).toHaveBeenCalledWith('m-desktop');
  });

  it('keeps a loaded conversation on a transient preview failure', async () => {
    previewMock.mockResolvedValueOnce([backendMessage({ body_loaded: true }) as never]);
    const store = useMessagesStore();
    await store.loadByThread('th-live');
    commandsAvailableMock.mockReturnValue(true);
    previewMock.mockRejectedValueOnce(new Error('offline'));
    await store.loadByThread('th-live');
    expect(store.getByThread('th-live').map(message => message.id)).toEqual(['m-live']);
    expect(useConnectionStore().backendErrorFor('messages')).toBe('offline');
  });

  it('revalidates changes received while the initial snapshot is still loading', async () => {
    let finish!: (value: Awaited<ReturnType<typeof backendLoadThreadMessages>>) => void;
    openMock.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    previewMock.mockResolvedValue([backendMessage({ id: 'arrived' }) as never]);
    const store = useMessagesStore();
    const initial = store.loadByThread('th-live');
    const refreshed = store.refreshThread('th-live');
    finish({ messages: [], bodies: [] });
    await Promise.all([initial, refreshed]);
    expect(previewMock).toHaveBeenCalledExactlyOnceWith('th-live');
    expect(store.getByThread('th-live').map(message => message.id)).toEqual(['arrived']);
  });

  it('coalesces refresh bursts and re-reads changes received during revalidation', async () => {
    vi.useFakeTimers();
    const store = useMessagesStore();
    openMock.mockResolvedValueOnce({ messages: [], bodies: [] });
    await store.loadByThread('th-live');
    let finish!: (value: never) => void;
    previewMock.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const first = store.refreshThread('th-live');
    const second = store.refreshThread('th-live');
    const third = store.refreshThread('th-live');
    previewMock.mockResolvedValueOnce([backendMessage({ id: 'latest', body_loaded: true }) as never]);
    finish([] as never);
    await Promise.all([first, second, third]);
    expect(previewMock).toHaveBeenCalledTimes(2);
    expect(store.getByThread('th-live').map(message => message.id)).toEqual(['latest']);
    vi.useRealTimers();
  });

  it('reports a failed conversation load and clears it on the next answer', async () => {
    commandsAvailableMock.mockReturnValue(true);
    previewMock.mockRejectedValueOnce(new Error('db is locked'));
    const messages = useMessagesStore();
    const connection = useConnectionStore();

    await messages.loadByThread('th-live');
    expect(connection.backendErrorFor('messages')).toBe('db is locked');

    // A thread that really holds nothing must clear it too, or the reader keeps
    // showing the old failure over an empty answer.
    previewMock.mockResolvedValueOnce([]);
    await messages.loadByThread('th-live');
    expect(connection.backendErrorFor('messages')).toBeNull();
  });

  it('delegates unsubscribe requests through the messages store', async () => {
    unsubscribeMock.mockResolvedValue({ method: 'https', target: 'https://example.com/unsubscribe' });
    const messages = useMessagesStore();

    await expect(messages.unsubscribe('m-unsubscribe')).resolves.toEqual({
      method: 'https',
      target: 'https://example.com/unsubscribe',
    });
    expect(unsubscribeMock).toHaveBeenCalledWith('m-unsubscribe');
  });

  it('clears a thread the backend reports as empty', async () => {
    previewMock.mockResolvedValue([]);
    const messages = useMessagesStore();
    messages.messagesByThread['th-gmail-empty'] = [
      {
        id: 'm-stale',
        threadId: 'th-gmail-empty',
        from: { name: 'Sender', email: 'sender@example.com' },
        to: [{ name: 'You', email: 'you@example.com' }],
        subject: 'Stale',
        bodyText: 'Stale body',
        timestamp: new Date().toISOString(),
        isRead: true,
        attachments: [],
      },
    ];

    await messages.loadByThread('th-gmail-empty');

    expect(messages.getByThread('th-gmail-empty')).toEqual([]);
  });

  it('preserves the draft marker from the backend preview', async () => {
    previewMock.mockResolvedValue([backendMessage({ is_draft: true }) as never]);
    bodiesMock.mockResolvedValue([
      {
        id: 'm-live',
        snippet: 'Preview only',
        body_text: 'Draft body',
        body_html: null,
        body_loaded: true,
      } as never,
    ]);
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');

    expect(messages.getByThread('th-live')[0]?.isDraft).toBe(true);
  });

  it('preserves Reply-To from the backend preview', async () => {
    previewMock.mockResolvedValue([
      backendMessage({ reply_to: 'replies@example.net' }) as never,
    ]);
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');

    expect(messages.getByThread('th-live')[0]?.replyTo).toBe('replies@example.net');
  });

  it('maps source metadata without treating malformed payloads as content', async () => {
    previewMock.mockResolvedValue([
      backendMessage({
        source_payload: JSON.stringify({ kind: 'rss', level: 'info', url: 'https://example.com/a' }),
      }) as never,
    ]);
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');

    expect(messages.getByThread('th-live')[0]?.sourcePayload).toEqual({
      kind: 'rss',
      level: 'info',
      url: 'https://example.com/a',
    });

    previewMock.mockResolvedValue([backendMessage({ source_payload: '{broken' }) as never]);
    await messages.loadByThread('th-second');
    expect(messages.getByThread('th-second')[0]?.sourcePayload).toBeUndefined();
  });

  it('shares duplicate loads until the visible message body is hydrated', async () => {
    previewMock.mockResolvedValue([backendMessage() as never]);
    let resolveBodies!: (value: never[]) => void;
    bodiesMock.mockImplementation(() => new Promise((resolve) => {
      resolveBodies = resolve;
    }));
    const messages = useMessagesStore();

    const firstLoad = messages.loadByThread('th-live');
    const duplicateLoad = messages.loadByThread('th-live');
    await vi.waitFor(() => expect(bodiesMock).toHaveBeenCalledTimes(1));

    expect(previewMock).toHaveBeenCalledTimes(1);

    resolveBodies([
      {
        id: 'm-live',
        snippet: 'Hydrated',
        body_text: 'Hydrated body',
        body_html: null,
        body_loaded: true,
        pgp_state: null,
        pgp_error_detail: null,
        pgp_signer_name: null,
        pgp_signer_fingerprint: null,
        smime_state: null,
        smime_signer_email: null,
        smime_signer_cn: null,
        smime_error_detail: null,
      },
    ] as never[]);
    await Promise.all([firstLoad, duplicateLoad]);

    expect(messages.getByThread('th-live')[0]?.bodyText).toBe('Hydrated body');
    expect(messages.getByThread('th-live')[0]?.bodyLoaded).toBe(true);
  });

  it('keeps an already rendered body visible while its preview refreshes', async () => {
    previewMock.mockResolvedValue([backendMessage() as never]);
    bodiesMock.mockResolvedValueOnce([
      {
        id: 'm-live',
        snippet: 'Hydrated',
        body_text: 'Stable body',
        body_html: null,
        body_loaded: true,
      },
    ] as never[]);
    const messages = useMessagesStore();
    await messages.loadByThread('th-live');

    let resolveRefresh!: (value: never[]) => void;
    bodiesMock.mockImplementationOnce(() => new Promise((resolve) => {
      resolveRefresh = resolve;
    }));
    const refresh = messages.loadByThread('th-live');
    await vi.waitFor(() => expect(bodiesMock).toHaveBeenCalledTimes(2));

    expect(messages.getByThread('th-live')[0]).toMatchObject({
      bodyText: 'Stable body',
      bodyLoaded: true,
    });

    resolveRefresh([
      {
        id: 'm-live',
        snippet: 'Refreshed',
        body_text: 'Refreshed body',
        body_html: null,
        body_loaded: true,
      },
    ] as never[]);
    await refresh;
    expect(messages.getByThread('th-live')[0]?.bodyText).toBe('Refreshed body');
  });

  it('hydrates trusted verdicts and clears old HTML and authentication on reparse', async () => {
    previewMock.mockResolvedValue([backendMessage() as never]);
    bodiesMock.mockResolvedValueOnce([{
      id: 'm-live', snippet: 'Old body', body_text: 'Old body',
      body_html: '<p>Old unsigned HTML</p>', body_loaded: true,
      auth_spf: 'pass', auth_dkim: 'pass', auth_dmarc: 'pass',
    }]);
    const messages = useMessagesStore();
    await messages.loadByThread('th-live');
    expect(messages.getByThread('th-live')[0]?.authDmarc).toBe('pass');

    bodiesMock.mockResolvedValueOnce([{
      id: 'm-live', snippet: 'Signed plain text', body_text: 'Signed plain text',
      body_html: null, body_loaded: true, pgp_state: 'signature_valid',
      auth_spf: null, auth_dkim: null, auth_dmarc: null,
    }]);
    await messages.loadByThread('th-live');
    expect(messages.getByThread('th-live')[0]).toMatchObject({
      bodyText: 'Signed plain text', pgp: { state: 'signature_valid' },
      authSpf: null, authDkim: null, authDmarc: null,
    });
    expect(messages.getByThread('th-live')[0]?.bodyHtml).toBeUndefined();
  });

  it('hydrates the preceding context together with an own latest reply', async () => {
    previewMock.mockResolvedValue([
      backendMessage({
        id: 'm-context',
        date: '2026-07-11T09:00:00Z',
      }) as never,
      backendMessage({
        id: 'm-own',
        from_address: 'tom@satchel.dev',
        from_name: '',
        date: '2026-07-11T10:00:00Z',
      }) as never,
    ]);
    bodiesMock.mockResolvedValue([
      {
        id: 'm-context',
        snippet: 'Question',
        body_text: 'What should we do?',
        body_html: null,
        body_loaded: true,
      },
      {
        id: 'm-own',
        snippet: 'Answer',
        body_text: 'I will handle it.',
        body_html: null,
        body_loaded: true,
      },
    ] as never[]);
    // The own-reply heuristic needs an identity to compare against.
    useAccountsStore().accounts = [{
      id: 'acc-work', displayName: 'Work', email: 'tom@satchel.dev',
      provider: 'imap', kind: 'email', color: '#3b82f6',
    }];
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');

    expect(bodiesMock).toHaveBeenCalledWith(['m-context', 'm-own']);
    expect(messages.getByThread('th-live').map((message) => message.bodyLoaded)).toEqual([true, true]);
  });

  it('defers hidden message bodies until the complete thread is requested', async () => {
    previewMock.mockResolvedValue([
      backendMessage({ id: 'm-old-1', date: '2026-07-11T08:00:00Z' }) as never,
      backendMessage({ id: 'm-old-2', date: '2026-07-11T09:00:00Z' }) as never,
      backendMessage({ id: 'm-latest', date: '2026-07-11T10:00:00Z' }) as never,
    ]);
    bodiesMock.mockImplementation(async (messageIds) =>
      messageIds.map((id) => ({
        id,
        snippet: String(id),
        body_text: `Body ${id}`,
        body_html: null,
        body_loaded: true,
      })) as never,
    );
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');

    // The open call brings the latest two; the oldest waits for an expansion.
    expect(openMock).toHaveBeenCalledTimes(1);
    expect(bodiesMock).toHaveBeenCalledTimes(1);
    expect(bodiesMock).toHaveBeenLastCalledWith(['m-old-2', 'm-latest']);
    expect(messages.getByThread('th-live').map((message) => message.bodyLoaded)).toEqual([
      false,
      true,
      true,
    ]);

    await messages.loadAttachmentsForMessages(
      'th-live',
      messages.getByThread('th-live').map((message) => message.id),
    );

    expect(bodiesMock).toHaveBeenNthCalledWith(2, ['m-old-1']);
    expect(messages.getByThread('th-live').every((message) => message.bodyLoaded)).toBe(true);
  });

  /*
   * PERF-09. A topic feed hydrated every linked thread in full, and the
   * attachment listing was one invoke per message: twenty threads of eight
   * mails came to more than 300 IPC rounds. One thread now costs a bounded
   * handful — the feed starts only the few it can see — and its attachment
   * listing is a single round trip however many messages it has.
   */
  /** PERF-15: the preview no longer carries the payload; the body brings it. */
  it('takes the source payload from the message body', async () => {
    previewMock.mockResolvedValue([backendMessage() as never]);
    bodiesMock.mockResolvedValue([{
      ...inlineBody('<p>Item</p>'),
      source_payload: JSON.stringify({ kind: 'rss', level: 'info', url: 'https://feed.example/item' }),
    }] as never[]);
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');

    await vi.waitFor(() => {
      expect(messages.getByThread('th-live')[0]?.sourcePayload).toEqual({
        kind: 'rss', level: 'info', url: 'https://feed.example/item',
      });
    });
  });

  it('takes the unsubscribe headers from the body and keeps them across a preview refresh', async () => {
    // The backend preview never carries them; only the body does.
    previewMock.mockResolvedValue([backendMessage() as never]);
    bodiesMock.mockResolvedValue([{
      ...inlineBody('<p>Newsletter</p>'),
      list_unsubscribe: '<https://news.example/u>',
      list_unsubscribe_post: 'List-Unsubscribe=One-Click',
    }] as never[]);
    const messages = useMessagesStore();
    await messages.loadByThread('th-live');
    await vi.waitFor(() => {
      expect(messages.getByThread('th-live')[0]).toMatchObject({
        listUnsubscribe: '<https://news.example/u>',
        listUnsubscribePost: 'List-Unsubscribe=One-Click',
      });
    });

    await messages.refreshThread('th-live');

    expect(messages.getByThread('th-live')[0]?.listUnsubscribe).toBe('<https://news.example/u>');
  });

  it('lists the attachments of a whole thread in one invoke', async () => {
    const ids = Array.from({ length: 8 }, (_, index) => `m-${index}`);
    previewMock.mockResolvedValue(ids.map((id, index) =>
      backendMessage({ id, date: `2026-07-11T0${index}:00:00Z` }) as never));
    bodiesMock.mockImplementation(async (messageIds) =>
      messageIds.map((id) => ({
        id, snippet: String(id), body_text: `Body ${id}`, body_html: null, body_loaded: true,
      })) as never);
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');
    await messages.loadAttachmentsForMessages('th-live', ids);

    const listedIds = attachmentsMock.mock.calls.flatMap(([messageIds]) => messageIds);
    expect(new Set(listedIds)).toEqual(new Set(ids));
    expect(attachmentsMock.mock.calls.length).toBeLessThanOrEqual(2);
    // Preview, bodies in chunks of three, attachment listings. Five visible
    // threads at this cost stay well under the 60 rounds the issue allows.
    const rounds = previewMock.mock.calls.length
      + bodiesMock.mock.calls.length
      + attachmentsMock.mock.calls.length;
    expect(rounds).toBeLessThanOrEqual(8);
  });

  it('leaves messages claimable when a newer load supersedes the listing', async () => {
    previewMock.mockResolvedValue([backendMessage() as never]);
    bodiesMock.mockResolvedValue([inlineBody('<img src="cid:image001.jpg@01DD16D4.C2220470">')] as never[]);
    let releaseListing!: (rows: never[]) => void;
    attachmentsMock.mockImplementationOnce(() => new Promise((resolve) => {
      releaseListing = resolve as (rows: never[]) => void;
    }));
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');
    await vi.waitFor(() => expect(attachmentsMock).toHaveBeenCalledTimes(1));
    // A reload bumps the thread version while the first listing is still out.
    attachmentsMock.mockResolvedValue([inlineAttachmentRow()] as never);
    downloadAttachmentMock.mockResolvedValue({
      filename: 'image001.jpg', mime_type: 'image/jpeg', data_base64: 'aW5saW5l', cached_path: null,
    });
    await messages.loadByThread('th-live');
    releaseListing([]);
    // The superseded pass has let go of the message, so asking again works.
    await vi.waitFor(async () => {
      await messages.loadAttachmentsForMessages('th-live', ['m-live']);
      expect(downloadAttachmentMock).toHaveBeenCalled();
    });

    await vi.waitFor(() => {
      expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toBe('data:image/jpeg;base64,aW5saW5l');
    });
  });

  it('keeps a thread nobody shows for the idle retention, then lets it go', async () => {
    vi.useFakeTimers();
    try {
      previewMock.mockImplementation(async (threadId) => [
        backendMessage({ id: `message-${threadId}`, thread_id: threadId }) as never,
      ]);
      bodiesMock.mockImplementation(async (messageIds) =>
        messageIds.map((id) => ({
          id, snippet: String(id), body_text: `Body ${id}`, body_html: null, body_loaded: true,
        })) as never,
      );
      const messages = useMessagesStore();

      for (const threadId of ['thread-1', 'thread-2', 'thread-3', 'thread-4', 'thread-5']) {
        await messages.loadByThread(threadId);
      }
      const release = messages.retainThread('thread-5');
      // Not three entries any more: five recent threads all stay.
      expect(messages.cacheStats().threadCount).toBe(5);

      await vi.advanceTimersByTimeAsync(THREAD_RETENTION_IDLE_MS + 100);
      expect(messages.getByThread('thread-1')).toEqual([]);
      expect(messages.getByThread('thread-5')).toHaveLength(1);

      release();
      await vi.advanceTimersByTimeAsync(THREAD_RETENTION_IDLE_MS + 100);
      expect(messages.getByThread('thread-5')).toEqual([]);
      expect(messages.cacheStats().threadCount).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('evicts older threads when full message bodies exceed the absolute cache budget', async () => {
    previewMock.mockImplementation(async (threadId) => [
      backendMessage({ id: `message-${threadId}`, thread_id: threadId }) as never,
    ]);
    bodiesMock.mockImplementation(async (messageIds) =>
      messageIds.map((id) => ({
        id,
        snippet: String(id),
        body_text: 'x'.repeat(7_000_000),
        body_html: null,
        body_loaded: true,
      })) as never,
    );
    const messages = useMessagesStore();

    await messages.loadByThread('large-thread-1');
    await messages.loadByThread('large-thread-2');

    expect(messages.getByThread('large-thread-1')).toEqual([]);
    expect(messages.getByThread('large-thread-2')).toHaveLength(1);
    expect(messages.cacheStats().bodyChars).toBeLessThanOrEqual(12_000_000);
  });

  it('uses validated image metadata when the attachment endpoint has no media type', async () => {
    const messages = useMessagesStore();
    previewMock.mockResolvedValue([backendMessage()] as never);
    bodiesMock.mockResolvedValue([inlineBody('<img src="cid:image001.jpg@01DD16D4.C2220470">')] as never);
    attachmentsMock.mockResolvedValue([inlineAttachmentRow()] as never);
    downloadAttachmentMock.mockResolvedValue({ filename: 'logo.png', mime_type: 'application/octet-stream', data_base64: 'AAAA', cached_path: null } as never);
    await messages.loadByThread('th-live');
    await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toBe('data:image/jpeg;base64,AAAA'));
  });

  it('rejects malicious MIME metadata returned by an inline download', async () => {
    const messages = useMessagesStore();
    previewMock.mockResolvedValue([backendMessage()] as never);
    bodiesMock.mockResolvedValue([inlineBody('<img src="cid:image001.jpg@01DD16D4.C2220470">')] as never);
    attachmentsMock.mockResolvedValue([inlineAttachmentRow()] as never);
    downloadAttachmentMock.mockResolvedValue({
      filename: 'logo.png', mime_type: 'image/png"><link rel=preconnect href=https://u1.t.example>',
      data_base64: 'AAAA', cached_path: null,
    } as never);
    await messages.loadByThread('th-live');
    await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalled());
    const doc = new DOMParser().parseFromString(messages.getByThread('th-live')[0]?.bodyHtml ?? '', 'text/html');
    expect(doc.querySelectorAll('link, meta')).toHaveLength(0);
    expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toBeFalsy();
  });

  it('hydrates CID images without creating persistent blob URLs', async () => {
    previewMock.mockResolvedValue([backendMessage() as never]);
    bodiesMock.mockResolvedValue([
      {
        id: 'm-live',
        snippet: 'Hydrated',
        body_text: 'Hydrated body',
        body_html: '<img src="cid:signature.png">',
        body_loaded: true,
        pgp_state: null,
        pgp_error_detail: null,
        pgp_signer_name: null,
        pgp_signer_fingerprint: null,
        smime_state: null,
        smime_signer_email: null,
        smime_signer_cn: null,
        smime_error_detail: null,
      },
    ] as never[]);
    attachmentsMock.mockResolvedValue([
      {
        id: 'att-inline',
        message_id: 'm-live',
        filename: 'signature.png',
        content_type: 'image/png',
        size: 1234,
        content_id: 'signature.png',
        is_inline: 1,
        cached_path: null,
      },
    ]);
    downloadAttachmentMock.mockResolvedValue({
      filename: 'signature.png',
      mime_type: 'image/png',
      data_base64: 'aW5saW5lLWltYWdl',
      cached_path: '/tmp/signature.png',
    });
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');
    await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalledWith('att-inline'));

    const message = messages.getByThread('th-live')[0];
    const attachment = message?.attachments[0];
    expect(attachment?.id).toBe('att-inline');
    expect(attachment?.dataUrl).toBe('data:image/png;base64,aW5saW5lLWltYWdl');
    // The bytes live once, on the attachment; the body keeps its reference.
    expect(message?.bodyHtml).toBe('<img src="cid:signature.png">');
    expect(bodyHtmlWithInlineImages(message!)).toContain('src="data:image/png;base64,aW5saW5lLWltYWdl"');
    expect(bodyHtmlWithInlineImages(message!)).not.toContain('Image%20unavailable');
  });

  it('fetches a cid image the body references even when the part looks like a plain attachment', async () => {
    previewMock.mockResolvedValue([backendMessage() as never]);
    bodiesMock.mockResolvedValue([
      inlineBody('<img src="cid:image001.jpg@01DD16D4.C2220470">'),
    ] as never[]);
    attachmentsMock.mockResolvedValue([
      // Outlook ships signature logos like this: no inline disposition, and a
      // content type the sender's gateway degraded to octet-stream.
      inlineAttachmentRow({ content_type: 'application/octet-stream', is_inline: 0 }),
    ]);
    downloadAttachmentMock.mockResolvedValue({
      filename: 'image001.jpg',
      mime_type: 'image/jpeg',
      data_base64: 'aW5saW5l',
      cached_path: null,
    });
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');
    await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalledWith('att-inline'));

    await vi.waitFor(() => {
      expect(bodyHtmlWithInlineImages(messages.getByThread('th-live')[0]!))
        .toContain('src="data:image/jpeg;base64,aW5saW5l"');
    });
  });

  it('keeps the cid reference when the download fails, and resolves it on retry', async () => {
    previewMock.mockResolvedValue([backendMessage() as never]);
    bodiesMock.mockResolvedValue([
      inlineBody('<img src="cid:image001.jpg@01DD16D4.C2220470">'),
    ] as never[]);
    attachmentsMock.mockResolvedValue([inlineAttachmentRow()]);
    downloadAttachmentMock.mockRejectedValueOnce(
      new Error('gmail attachment fetch skipped: rate limited for another 812s'),
    );
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');
    await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalledTimes(1));

    await vi.waitFor(() => {
      const failed = messages.getByThread('th-live')[0];
      // The cid: URL survives a transient provider error — a placeholder here
      // would destroy the only pointer a retry has.
      expect(failed?.bodyHtml).toContain('src="cid:image001.jpg@01DD16D4.C2220470"');
      expect(failed?.bodyHtml).not.toContain('Image%20unavailable');
      expect(failed?.inlineImagesPending).toBe(1);
      expect(failed?.inlineImagesError).toContain('rate limited');
    });

    downloadAttachmentMock.mockResolvedValue({
      filename: 'image001.jpg',
      mime_type: 'image/jpeg',
      data_base64: 'aW5saW5l',
      cached_path: null,
    });
    await messages.retryInlineImages('th-live', 'm-live');

    const healed = messages.getByThread('th-live')[0];
    expect(bodyHtmlWithInlineImages(healed!)).toContain('src="data:image/jpeg;base64,aW5saW5l"');
    expect(healed?.inlineImagesPending).toBe(0);
    expect(healed?.inlineImagesError).toBeUndefined();
  });

  it('shows the placeholder only when the referenced part is gone for good', async () => {
    previewMock.mockResolvedValue([backendMessage() as never]);
    bodiesMock.mockResolvedValue([inlineBody('<img src="cid:vanished@example.com">')] as never[]);
    attachmentsMock.mockResolvedValue([]);
    const messages = useMessagesStore();

    await messages.loadByThread('th-live');

    await vi.waitFor(() => {
      const message = messages.getByThread('th-live')[0];
      expect(message?.inlineImagesPending).toBe(0);
      expect(bodyHtmlWithInlineImages(message!)).toContain('Image%20unavailable');
    });
    expect(downloadAttachmentMock).not.toHaveBeenCalled();
  });
});

describe('opening, refreshing and prefetching a thread', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    commandsAvailableMock.mockReturnValue(true);
    previewMock.mockReset();
    bodiesMock.mockReset();
    attachmentsMock.mockReset();
    attachmentsMock.mockResolvedValue([]);
    downloadAttachmentMock.mockReset();
    loadBodyMock.mockReset();
    openMock.mockClear();
  });

  function seedThread(ids: string[]) {
    previewMock.mockImplementation(async () => ids.map((id, index) =>
      backendMessage({ id, date: `2026-07-11T0${index}:00:00Z` }) as never));
    bodiesMock.mockImplementation(async (messageIds) => messageIds.map((id) => ({
      ...inlineBody('<img src="cid:image001.jpg@01DD16D4.C2220470">'),
      id,
    })) as never);
    attachmentsMock.mockImplementation(async (messageIds) =>
      messageIds.map((id) => inlineAttachmentRow({ id: `att-${id}`, message_id: id })) as never);
    downloadAttachmentMock.mockResolvedValue({
      filename: 'image001.jpg', mime_type: 'image/jpeg', data_base64: 'aW5saW5l', cached_path: null,
    });
  }

  it('paints from one call and reopens a cached thread without any', async () => {
    seedThread(['m-1', 'm-2']);
    const messages = useMessagesStore();

    await messages.openThread('th-live');
    await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalledTimes(1));
    expect(openMock).toHaveBeenCalledTimes(1);
    expect(previewMock).toHaveBeenCalledTimes(1);

    await messages.openThread('th-live');
    expect(openMock).toHaveBeenCalledTimes(1);
    expect(previewMock).toHaveBeenCalledTimes(1);
    expect(attachmentsMock).toHaveBeenCalledTimes(1);
  });

  it('refreshes previews only and keeps bodies and inline images', async () => {
    seedThread(['m-1', 'm-2']);
    const messages = useMessagesStore();
    await messages.openThread('th-live');
    await vi.waitFor(() => expect(messages.getByThread('th-live')[1]?.attachments[0]?.dataUrl).toBeTruthy());
    const bodyCalls = bodiesMock.mock.calls.length;

    await messages.refreshThread('th-live');

    expect(previewMock).toHaveBeenCalledTimes(2);
    expect(bodiesMock).toHaveBeenCalledTimes(bodyCalls);
    expect(downloadAttachmentMock).toHaveBeenCalledTimes(1);
    const latest = messages.getByThread('th-live')[1]!;
    expect(latest.bodyLoaded).toBe(true);
    expect(latest.attachments[0]?.dataUrl).toBe('data:image/jpeg;base64,aW5saW5l');

    // A reply that arrived since: only its body and images are fetched.
    seedThread(['m-1', 'm-2', 'm-3']);
    await messages.refreshThread('th-live');
    await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalledTimes(2));
    expect(bodiesMock).toHaveBeenLastCalledWith(['m-3']);
    expect(attachmentsMock).toHaveBeenLastCalledWith(['m-3']);
  });

  it('reloads a stale cached thread on open, behind the cached view', async () => {
    seedThread(['m-1']);
    const messages = useMessagesStore();
    await messages.openThread('th-live');
    messages.markThreadsStale();

    const reopening = messages.openThread('th-live');
    expect(messages.getByThread('th-live')).toHaveLength(1);
    await reopening;
    expect(previewMock).toHaveBeenCalledTimes(2);
    expect(openMock).toHaveBeenCalledTimes(1);
  });

  it('prefetches from local data only and finishes the job when opened', async () => {
    previewMock.mockResolvedValue([backendMessage({ id: 'm-old' })] as never);
    bodiesMock.mockResolvedValue([
      { id: 'm-old', snippet: '', body_text: '', body_html: null, body_loaded: false },
    ] as never);
    loadBodyMock.mockResolvedValue({ body_text: 'Vom Server', body_html: null, fetched: true });
    const messages = useMessagesStore();

    await messages.prefetchThread('th-live');
    expect(messages.hasThread('th-live')).toBe(true);
    expect(messages.isLoading('th-live')).toBe(false);
    expect(loadBodyMock).not.toHaveBeenCalled();
    expect(attachmentsMock).not.toHaveBeenCalled();

    await messages.openThread('th-live');
    expect(openMock).toHaveBeenCalledTimes(1);
    expect(loadBodyMock).toHaveBeenCalledWith('m-old');
    expect(messages.getByThread('th-live')[0]?.bodyText).toBe('Vom Server');
  });
});

/**
 * A phone can keep only the header of an older message, and legacy versions
 * could keep only the first 64 KiB. Both need the complete body from the
 * server instead of becoming a blank or cut-off page.
 */
describe('a message whose full body is not stored', () => {
  beforeEach(() => {
    loadBodyMock.mockReset();
  });

  it('fetches it from the server and shows it', async () => {
    previewMock.mockResolvedValue([backendMessage({ id: 'm-old' })] as never);
    bodiesMock.mockResolvedValue([
      { id: 'm-old', snippet: '', body_text: '', body_html: null, body_loaded: true },
    ] as never);
    loadBodyMock.mockResolvedValue({
      body_text: 'Nachgeladen vom Server',
      body_html: null,
      fetched: true,
    });

    const store = useMessagesStore();
    await store.loadByThread('th-live');
    await vi.waitFor(() => expect(loadBodyMock).toHaveBeenCalledWith('m-old'));
    await vi.waitFor(() =>
      expect(store.messagesByThread['th-live']?.[0]?.bodyText).toBe('Nachgeladen vom Server'),
    );
  });

  it('fetches it when the backend supplied only a preview fallback', async () => {
    previewMock.mockResolvedValue([backendMessage({ id: 'm-truncated' })] as never);
    bodiesMock.mockResolvedValue([
      {
        id: 'm-truncated',
        snippet: 'Preview only',
        body_text: 'Preview only',
        body_html: null,
        body_loaded: false,
      },
    ] as never);
    loadBodyMock.mockResolvedValue({
      body_text: 'Vollständig nachgeladen',
      body_html: '<p>Vollständig nachgeladen</p>',
      fetched: true,
    });

    const store = useMessagesStore();
    await store.loadByThread('th-live');

    await vi.waitFor(() => expect(loadBodyMock).toHaveBeenCalledWith('m-truncated'));
    await vi.waitFor(() =>
      expect(store.messagesByThread['th-live']?.[0]?.bodyHtml)
        .toBe('<p>Vollständig nachgeladen</p>'),
    );
  });

  /** A body that is already there must not cost a network round trip. */
  it('leaves a stored body alone', async () => {
    previewMock.mockResolvedValue([backendMessage({ id: 'm-new' })] as never);
    bodiesMock.mockResolvedValue([
      { id: 'm-new', snippet: '', body_text: 'Schon da', body_html: null, body_loaded: true },
    ] as never);

    const store = useMessagesStore();
    await store.loadByThread('th-live');
    expect(loadBodyMock).not.toHaveBeenCalled();
  });

  it('says so when the server cannot supply it either', async () => {
    previewMock.mockResolvedValue([backendMessage({ id: 'm-gone' })] as never);
    bodiesMock.mockResolvedValue([
      { id: 'm-gone', snippet: '', body_text: '', body_html: null, body_loaded: true },
    ] as never);
    loadBodyMock.mockRejectedValue(new Error('Diese Nachricht liegt auf keinem Server mehr.'));

    const store = useMessagesStore();
    await store.loadByThread('th-live');
    await vi.waitFor(() =>
      expect(store.messagesByThread['th-live']?.[0]?.bodyLoadError).toContain('keinem Server'),
    );
  });

  it('uses a body returned as already cached instead of leaving it loading', async () => {
    previewMock.mockResolvedValue([backendMessage({ id: 'm-cached-remotely' })] as never);
    bodiesMock.mockResolvedValue([
      { id: 'm-cached-remotely', snippet: '', body_text: '', body_html: null, body_loaded: false },
    ] as never);
    loadBodyMock.mockResolvedValue({
      body_text: 'Schon vollständig vorhanden',
      body_html: null,
      fetched: false,
    });

    const store = useMessagesStore();
    await store.loadByThread('th-live');
    await vi.waitFor(() =>
      expect(store.messagesByThread['th-live']?.[0]?.bodyText)
        .toBe('Schon vollständig vorhanden'),
    );

    expect(store.messagesByThread['th-live']?.[0]?.bodyLoaded).toBe(true);
  });

  it('does not start a second remote fetch for the same visible body', async () => {
    previewMock.mockResolvedValue([backendMessage({ id: 'm-pending' })] as never);
    bodiesMock.mockResolvedValue([
      { id: 'm-pending', snippet: '', body_text: '', body_html: null, body_loaded: false },
    ] as never);
    let resolveBody!: (body: { body_text: string; body_html: null; fetched: true }) => void;
    loadBodyMock.mockImplementation(() => new Promise((resolve) => {
      resolveBody = resolve;
    }));

    const store = useMessagesStore();
    const initialLoad = store.loadByThread('th-live');
    await vi.waitFor(() => expect(store.isBodyLoading('m-pending')).toBe(true));
    const duplicateLoad = store.loadByThread('th-live');

    expect(loadBodyMock).toHaveBeenCalledTimes(1);
    expect(store.isLoading('th-live')).toBe(true);
    resolveBody({ body_text: 'Fertig', body_html: null, fetched: true });
    await Promise.all([initialLoad, duplicateLoad]);
    await vi.waitFor(() => expect(store.isBodyLoading('m-pending')).toBe(false));
    expect(store.messagesByThread['th-live']?.[0]?.bodyText).toBe('Fertig');
  });

  it('stops showing a loading state when the remote body request hangs', async () => {
    vi.useFakeTimers();
    try {
      previewMock.mockResolvedValue([backendMessage({ id: 'm-hanging-draft', is_draft: true })] as never);
      bodiesMock.mockResolvedValue([
        { id: 'm-hanging-draft', snippet: '', body_text: '', body_html: null, body_loaded: true },
      ] as never);
      loadBodyMock.mockImplementation(() => new Promise(() => {}));

      const store = useMessagesStore();
      const loading = store.loadByThread('th-live');
      await vi.advanceTimersByTimeAsync(30_000);
      await loading;

      expect(store.messagesByThread['th-live']?.[0]?.bodyLoaded).toBe(true);
      expect(store.messagesByThread['th-live']?.[0]?.bodyLoadError).toContain('took too long');
    } finally {
      vi.useRealTimers();
    }
  });
});
