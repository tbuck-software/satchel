// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendCommandsAvailable,
  backendDownloadAttachment,
  backendGetMessageSource,
  backendChooseSavePath,
  backendExportMessage,
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
  backendChooseSavePath: vi.fn(),
  backendExportMessage: vi.fn(),
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

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
    vi.mocked(backendChooseSavePath).mockReset();
    vi.mocked(backendExportMessage).mockReset();
    unsubscribeMock.mockReset();
  });

  it('exports complete EML through the backend rather than rebuilding display data', async () => {
    commandsAvailableMock.mockReturnValue(true);
    vi.mocked(backendChooseSavePath).mockResolvedValue('/exports/message.eml');
    vi.mocked(backendExportMessage).mockResolvedValue(undefined);
    const messages = useMessagesStore();
    await expect(messages.exportMessage('message-original', 'Report / original')).resolves.toBe(true);
    expect(backendChooseSavePath).toHaveBeenCalledWith(expect.objectContaining({
      suggestedName: 'Report original.eml', extensions: ['eml'],
    }));
    expect(backendExportMessage).toHaveBeenCalledExactlyOnceWith({ path: '/exports/message.eml', messageId: 'message-original' });
  });

  it('does not write an EML when the save dialog is cancelled', async () => {
    commandsAvailableMock.mockReturnValue(true);
    vi.mocked(backendChooseSavePath).mockResolvedValue(null);
    await expect(useMessagesStore().exportMessage('message', 'Subject')).resolves.toBe(false);
    expect(backendExportMessage).not.toHaveBeenCalled();
  });

  it('reprocesses cached PGP bodies after keys change, including older expanded mail', async () => {
    const armor = '-----BEGIN PGP MESSAGE-----\nciphertext\n-----END PGP MESSAGE-----';
    previewMock.mockResolvedValue([
      backendMessage({ id: 'encrypted-old', body_text: armor, body_loaded: true }) as never,
      backendMessage({ id: 'encrypted-new', body_text: armor, body_loaded: true }) as never,
      backendMessage({ id: 'plain', body_text: 'Unrelated', body_loaded: true }) as never,
    ]);
    const messages = useMessagesStore();
    await messages.loadByThread('th-live');
    bodiesMock.mockClear();
    bodiesMock.mockResolvedValue([
      { ...inlineBody(''), id: 'encrypted-old', body_text: 'Decrypted older mail', pgp_state: 'decrypted' },
      { ...inlineBody(''), id: 'encrypted-new', body_text: 'Decrypted latest mail', pgp_state: 'decrypted' },
    ]);

    await messages.reloadPgpMessages();
    expect(bodiesMock).toHaveBeenCalledExactlyOnceWith(['encrypted-old', 'encrypted-new']);
    expect(messages.getByThread('th-live').map((message) => message.bodyText))
      .toEqual(['Decrypted older mail', 'Decrypted latest mail', 'Unrelated']);
    await messages.openThread('th-live');
    expect(messages.getByThread('th-live')[0]?.pgp?.state).toBe('decrypted');
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

  it('releases request bookkeeping after many threads expire', async () => {
    vi.useFakeTimers();
    try {
      previewMock.mockImplementation(async (threadId) => [
        backendMessage({ id: `message-${threadId}`, thread_id: threadId }) as never,
      ]);
      bodiesMock.mockImplementation(async (ids) => ids.map((id) => ({
        ...inlineBody(''), id,
      })));
      const messages = useMessagesStore();
      for (let batch = 0; batch < 20; batch += 1) {
        for (let i = 0; i < 10; i += 1) await messages.loadByThread(`visited-${batch}-${i}`);
        await vi.advanceTimersByTimeAsync(THREAD_RETENTION_IDLE_MS + 100);
      }
      expect(messages.cacheStats()).toMatchObject({
        threadCount: 0, messageCount: 0, bodyChars: 0, requestGenerationCount: 0,
      });
      await messages.openThread('visited-0-0');
      expect(messages.getByThread('visited-0-0')).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it.each(['resolve', 'reject'] as const)('ignores a stale body %s after budget eviction and reopening', async (outcome) => {
    const oldBodies = deferred<Awaited<ReturnType<typeof backendListMessageBodies>>>();
    previewMock.mockResolvedValue([
      backendMessage({ id: 'hidden' }) as never,
      backendMessage({ id: 'middle' }) as never,
      backendMessage() as never,
    ]);
    bodiesMock.mockResolvedValue([inlineBody('')] as never);
    const messages = useMessagesStore();
    await messages.loadByThread('th-live');
    bodiesMock.mockReturnValueOnce(oldBodies.promise);
    const expanded = messages.loadAttachmentsForMessages('th-live', ['hidden']);
    // Evict while the expanded message's body request is still in flight.
    previewMock.mockResolvedValueOnce([backendMessage({ id: 'large', thread_id: 'large-thread' })] as never);
    bodiesMock.mockResolvedValueOnce([{ ...inlineBody(''), id: 'large', body_text: 'x'.repeat(12_000_001) }] as never);
    await messages.loadByThread('large-thread');
    expect(messages.hasThread('th-live')).toBe(false);
    previewMock.mockResolvedValueOnce([backendMessage({ id: 'hidden' })] as never);
    bodiesMock.mockResolvedValueOnce([{
      ...inlineBody(''), id: 'hidden', body_text: 'Current body',
      source_payload: JSON.stringify({ kind: 'rss', level: 'info', url: 'https://example.com/current' }),
    }] as never);
    await messages.openThread('th-live');
    if (outcome === 'resolve') oldBodies.resolve([{
      ...inlineBody(''), id: 'hidden', body_text: 'Stale body',
      source_payload: JSON.stringify({ kind: 'rss', level: 'info', url: 'https://example.com/stale' }),
    }] as never);
    else oldBodies.reject(new Error('Old load failed'));
    await expanded;
    expect(messages.getByThread('th-live')[0]).toMatchObject({
      bodyText: 'Current body', bodyLoaded: true, bodyLoadError: undefined,
      sourcePayload: { url: 'https://example.com/current' },
    });
  });

  it('keeps original-source requests independent of cache eviction and reopening', async () => {
    vi.useFakeTimers();
    try {
      commandsAvailableMock.mockReturnValue(true);
      previewMock.mockResolvedValue([backendMessage()] as never);
      bodiesMock.mockResolvedValue([inlineBody('')] as never);
      const messages = useMessagesStore();
      await messages.loadByThread('th-live');
      const original = deferred<Awaited<ReturnType<typeof backendGetMessageSource>>>();
      messageSourceMock.mockReturnValueOnce(original.promise);
      const sourceRequest = messages.loadMessageSource('m-live');
      await vi.advanceTimersByTimeAsync(THREAD_RETENTION_IDLE_MS + 100);
      expect(messages.hasThread('th-live')).toBe(false);
      bodiesMock.mockResolvedValueOnce([{ ...inlineBody(''), body_text: 'Current display' }] as never);
      await messages.openThread('th-live');
      const source = { message_id: 'm-live', raw_source: 'Subject: Original\r\n\r\nOriginal bytes' } as never;
      original.resolve(source);
      await expect(sourceRequest).resolves.toEqual(source);
      expect(messages.getByThread('th-live')[0]?.bodyText).toBe('Current display');
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not let an evicted remote body finalizer clear a reopened load', async () => {
    previewMock.mockResolvedValue([
      backendMessage({ id: 'hidden' }) as never,
      backendMessage({ id: 'middle' }) as never,
      backendMessage() as never,
    ]);
    bodiesMock.mockResolvedValue([inlineBody('')] as never);
    const messages = useMessagesStore();
    await messages.loadByThread('th-live');
    const oldBody = deferred<Awaited<ReturnType<typeof backendLoadMessageBody>>>();
    const newBody = deferred<Awaited<ReturnType<typeof backendLoadMessageBody>>>();
    loadBodyMock.mockReset();
    loadBodyMock.mockReturnValueOnce(oldBody.promise).mockReturnValueOnce(newBody.promise);
    bodiesMock.mockResolvedValueOnce([{ ...inlineBody(''), id: 'hidden', body_text: '', body_loaded: false }] as never);
    const oldExpanded = messages.loadAttachmentsForMessages('th-live', ['hidden']);
    await vi.waitFor(() => expect(loadBodyMock).toHaveBeenCalledTimes(1));
    previewMock.mockResolvedValueOnce([backendMessage({ id: 'large', thread_id: 'large-thread' })] as never);
    bodiesMock.mockResolvedValueOnce([{ ...inlineBody(''), id: 'large', body_text: 'x'.repeat(12_000_001) }] as never);
    await messages.loadByThread('large-thread');
    await messages.openThread('th-live');
    bodiesMock.mockResolvedValueOnce([{ ...inlineBody(''), id: 'hidden', body_text: '', body_loaded: false }] as never);
    const newExpanded = messages.loadAttachmentsForMessages('th-live', ['hidden']);
    await vi.waitFor(() => expect(loadBodyMock).toHaveBeenCalledTimes(2));
    oldBody.resolve({ body_text: 'Old remote body', body_html: null } as never);
    await oldExpanded;
    expect(messages.isBodyLoading('hidden')).toBe(true);
    newBody.resolve({ body_text: 'New remote body', body_html: null } as never);
    await newExpanded;
    expect(messages.isBodyLoading('hidden')).toBe(false);
    expect(messages.getByThread('th-live')[0]?.bodyText).toBe('New remote body');
  });

  it('keeps a reopened inline download owned by its current request', async () => {
    vi.useFakeTimers();
    try {
      previewMock.mockResolvedValue([backendMessage()] as never);
      bodiesMock.mockResolvedValue([inlineBody('<img src="cid:logo">')] as never);
      attachmentsMock.mockResolvedValue([inlineAttachmentRow({ content_id: 'logo' })] as never);
      const oldDownload = deferred<Awaited<ReturnType<typeof backendDownloadAttachment>>>();
      const newDownload = deferred<Awaited<ReturnType<typeof backendDownloadAttachment>>>();
      downloadAttachmentMock.mockReturnValueOnce(oldDownload.promise).mockReturnValueOnce(newDownload.promise);
      const messages = useMessagesStore();
      await messages.loadByThread('th-live');
      await vi.advanceTimersByTimeAsync(0);
      expect(downloadAttachmentMock).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(THREAD_RETENTION_IDLE_MS + 100);
      expect(messages.hasThread('th-live')).toBe(false);
      await messages.openThread('th-live');
      await vi.advanceTimersByTimeAsync(0);
      expect(downloadAttachmentMock).toHaveBeenCalledTimes(2);
      oldDownload.resolve({ filename: 'old.jpg', mime_type: 'image/jpeg', data_base64: 'OLD', cached_path: null });
      await vi.advanceTimersByTimeAsync(0);
      expect(messages.getByThread('th-live')[0]?.attachments).toEqual([]);
      expect(messages.needsDetails('m-live')).toBe(false);
      newDownload.resolve({ filename: 'new.jpg', mime_type: 'image/jpeg', data_base64: 'NEW', cached_path: null });
      await vi.advanceTimersByTimeAsync(0);
      expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toContain('NEW');
      expect(messages.needsDetails('m-live')).toBe(false);
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

  it.each([[7, 8], [undefined, 7], [7, undefined]])('discards body and inline caches when saved generation changes from %s to %s', async (oldGeneration, newGeneration) => {
    const html = '<img src="cid:image001.jpg@01DD16D4.C2220470">';
    previewMock.mockResolvedValue([backendMessage({ remote_uidvalidity: oldGeneration })] as never);
    bodiesMock.mockResolvedValue([{ ...inlineBody(html), remote_uidvalidity: oldGeneration }] as never);
    attachmentsMock.mockResolvedValue([inlineAttachmentRow()] as never);
    downloadAttachmentMock.mockResolvedValue({ filename: 'old.jpg', mime_type: 'image/jpeg', data_base64: 'b2xk', remote_uidvalidity: oldGeneration } as never);
    const messages = useMessagesStore();
    await messages.openThread('th-live');
    await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toContain('b2xk'));

    previewMock.mockResolvedValue([backendMessage({ remote_uidvalidity: newGeneration })] as never);
    bodiesMock.mockResolvedValue([{ ...inlineBody(html), body_text: 'Verified new body', remote_uidvalidity: newGeneration }] as never);
    downloadAttachmentMock.mockResolvedValue({ filename: 'new.jpg', mime_type: 'image/jpeg', data_base64: 'bmV3', remote_uidvalidity: newGeneration } as never);
    await messages.refreshThread('th-live');
    await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toContain('bmV3'));
    expect(messages.getByThread('th-live')[0]?.bodyText).toBe('Verified new body');
    expect(downloadAttachmentMock).toHaveBeenCalledTimes(2);
  });

  it('keeps a verified body and inline cache when the saved generation still matches', async () => {
    previewMock.mockResolvedValue([backendMessage({ remote_uidvalidity: 7 })] as never);
    bodiesMock.mockResolvedValue([{ ...inlineBody('<img src="cid:image001.jpg@01DD16D4.C2220470">'), remote_uidvalidity: 7 }] as never);
    attachmentsMock.mockResolvedValue([inlineAttachmentRow()] as never);
    downloadAttachmentMock.mockResolvedValue({ filename: 'image.jpg', mime_type: 'image/jpeg', data_base64: 'b2xk', remote_uidvalidity: 7 } as never);
    const messages = useMessagesStore();
    await messages.openThread('th-live');
    await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toContain('b2xk'));
    await messages.refreshThread('th-live');
    expect(messages.getByThread('th-live')[0]?.bodyText).toBe('Hydrated body');
    expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toContain('b2xk');
    expect(bodiesMock).toHaveBeenCalledTimes(1);
    expect(downloadAttachmentMock).toHaveBeenCalledTimes(1);
  });

  it.each([[7, 8], [8, 7]])('rejects a body snapshot from generation %s/%s until its preview is refreshed', async (previewGeneration, responseGeneration) => {
    let releasePreview!: (rows: never) => void;
    previewMock.mockResolvedValueOnce([backendMessage({ subject: 'Old envelope', remote_uidvalidity: previewGeneration })] as never);
    previewMock.mockImplementationOnce(() => new Promise(resolve => { releasePreview = resolve; }));
    bodiesMock.mockResolvedValueOnce([{ ...inlineBody(''), body_text: 'Unmatched body', remote_uidvalidity: responseGeneration }] as never);
    bodiesMock.mockResolvedValue([{ ...inlineBody(''), body_text: 'New body', remote_uidvalidity: 8 }] as never);
    const messages = useMessagesStore();
    await messages.openThread('th-live');
    await vi.waitFor(() => expect(previewMock).toHaveBeenCalledTimes(2));
    expect(messages.getByThread('th-live')[0]?.bodyText).not.toBe('Unmatched body');
    expect(messages.getByThread('th-live')[0]?.subject).toBe('Old envelope');

    releasePreview([backendMessage({ subject: 'New envelope', remote_uidvalidity: 8 })] as never);
    await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.bodyText).toBe('New body'));
    expect(messages.getByThread('th-live')[0]?.subject).toBe('New envelope');
  });

  it.each([[7, 8], [8, 7]])('rejects a remote body from generation %s/%s until its preview is refreshed', async (previewGeneration, responseGeneration) => {
    let releasePreview!: (rows: never) => void;
    previewMock.mockResolvedValueOnce([backendMessage({ remote_uidvalidity: previewGeneration })] as never);
    previewMock.mockImplementationOnce(() => new Promise(resolve => { releasePreview = resolve; }));
    bodiesMock.mockResolvedValueOnce([{ ...inlineBody(''), body_text: '', body_loaded: false, remote_uidvalidity: previewGeneration }] as never);
    bodiesMock.mockResolvedValue([{ ...inlineBody(''), body_text: 'Verified new body', remote_uidvalidity: 8 }] as never);
    loadBodyMock.mockResolvedValue({ body_text: 'Unmatched body', body_html: null, fetched: true, remote_uidvalidity: responseGeneration });
    const messages = useMessagesStore();
    await messages.openThread('th-live');
    await vi.waitFor(() => expect(previewMock).toHaveBeenCalledTimes(2));
    expect(messages.getByThread('th-live')[0]?.bodyText).not.toBe('Unmatched body');
    expect(messages.isBodyLoading('m-live')).toBe(false);

    releasePreview([backendMessage({ remote_uidvalidity: 8 })] as never);
    await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.bodyText).toBe('Verified new body'));
  });

  it.each([[7, 8], [8, 7]])('rejects an inline download from generation %s/%s before publishing its data URL', async (previewGeneration, responseGeneration) => {
    const html = '<img src="cid:image001.jpg@01DD16D4.C2220470">';
    let releasePreview!: (rows: never) => void;
    previewMock.mockResolvedValueOnce([backendMessage({ remote_uidvalidity: previewGeneration })] as never);
    previewMock.mockImplementationOnce(() => new Promise(resolve => { releasePreview = resolve; }));
    bodiesMock.mockResolvedValue([{ ...inlineBody(html), remote_uidvalidity: previewGeneration }] as never);
    attachmentsMock.mockResolvedValue([inlineAttachmentRow()] as never);
    downloadAttachmentMock.mockResolvedValueOnce({ filename: 'unmatched.jpg', mime_type: 'image/jpeg', data_base64: 'b2xk', remote_uidvalidity: responseGeneration } as never);
    downloadAttachmentMock.mockResolvedValue({ filename: 'new.jpg', mime_type: 'image/jpeg', data_base64: 'bmV3', remote_uidvalidity: 8 } as never);
    const messages = useMessagesStore();
    await messages.openThread('th-live');
    await vi.waitFor(() => expect(previewMock).toHaveBeenCalledTimes(2));
    expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toBeUndefined();

    bodiesMock.mockResolvedValue([{ ...inlineBody(html), remote_uidvalidity: 8 }] as never);
    releasePreview([backendMessage({ remote_uidvalidity: 8 })] as never);
    await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toContain('bmV3'));
  });

  it('does not let an old inline request overwrite a new generation or release its claim', async () => {
    const html = '<img src="cid:image001.jpg@01DD16D4.C2220470">';
    let finishOld!: (value: never) => void;
    let finishNew!: (value: never) => void;
    previewMock.mockResolvedValue([backendMessage({ remote_uidvalidity: 7 })] as never);
    bodiesMock.mockResolvedValue([{ ...inlineBody(html), remote_uidvalidity: 7 }] as never);
    attachmentsMock.mockResolvedValue([inlineAttachmentRow()] as never);
    downloadAttachmentMock.mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve; }));
    downloadAttachmentMock.mockImplementationOnce(() => new Promise(resolve => { finishNew = resolve; }));
    const messages = useMessagesStore();
    await messages.openThread('th-live');
    await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalledTimes(1));

    previewMock.mockResolvedValue([backendMessage({ remote_uidvalidity: 8 })] as never);
    bodiesMock.mockResolvedValue([{ ...inlineBody(html), remote_uidvalidity: 8 }] as never);
    await messages.refreshThread('th-live');
    await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalledTimes(2));
    finishOld({ filename: 'old.jpg', mime_type: 'image/jpeg', data_base64: 'b2xk', remote_uidvalidity: 7 } as never);
    await messages.loadAttachmentsForMessages('th-live', ['m-live']);
    expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toBeUndefined();
    expect(messages.needsDetails('m-live')).toBe(false);
    expect(downloadAttachmentMock).toHaveBeenCalledTimes(2);

    finishNew({ filename: 'new.jpg', mime_type: 'image/jpeg', data_base64: 'bmV3', remote_uidvalidity: 8 } as never);
    await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toContain('bmV3'));
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

  it.each(['imapUidValidityChanged', 'imapUidValidityMissing'])('keeps localized recovery when body fetch fails with %s', async (key) => {
    previewMock.mockResolvedValue([backendMessage({ id: 'm-generation' })] as never);
    bodiesMock.mockResolvedValue([
      { id: 'm-generation', snippet: '', body_text: '', body_html: null, body_loaded: true },
    ] as never);
    loadBodyMock.mockRejectedValue({ key: `backend.errors.${key}`, params: { folder: 'INBOX' } });
    const store = useMessagesStore();
    await store.loadByThread('th-live');
    const error = store.messagesByThread['th-live']?.[0]?.bodyLoadError;
    expect(error).toContain('INBOX');
    expect(error).toMatch(/sync.*folder/i);
    expect(error).not.toContain('backend.errors');
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

describe('content reference follow-ups', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    commandsAvailableMock.mockReturnValue(true);
    previewMock.mockReset();
    bodiesMock.mockReset();
    attachmentsMock.mockReset().mockResolvedValue([]);
    loadBodyMock.mockReset();
  });
it.each([null, 7])('invalidates displayed bytes when validity changes with saved generation %s unchanged', async (generation) => {
  previewMock.mockResolvedValue([backendMessage({ remote_uidvalidity: generation })] as never);
  bodiesMock.mockResolvedValue([{ ...inlineBody('<p>old bytes</p>'), remote_uidvalidity: generation }] as never);
  const messages = useMessagesStore();
  await messages.openThread('th-live');
  const content_error = JSON.stringify({ key: generation === null ? 'backend.errors.imapUidValidityMissing' : 'backend.errors.imapUidValidityChanged', params: { folder: 'INBOX' } });
  previewMock.mockResolvedValue([backendMessage({ remote_uidvalidity: generation, content_error, body_loaded: true, body_text: '' })] as never);
  bodiesMock.mockResolvedValue([{ ...inlineBody(''), remote_uidvalidity: generation, content_error, body_text: '', body_loaded: true }] as never);
  await messages.refreshThread('th-live');
  expect(messages.getByThread('th-live')[0]?.bodyHtml).toBeUndefined();
  expect(messages.getByThread('th-live')[0]?.attachments).toEqual([]);
  expect(messages.getByThread('th-live')[0]?.bodyLoadError).toMatch(/sync.*folder/i);
  expect(loadBodyMock).not.toHaveBeenCalled();
});

it('rehydrates an explicitly expanded older message after revalidating its reference without an ID watcher rerun', async () => {
  const { watch, nextTick } = await import('vue');
  let generation = 7;
  const previews = () => ['old', 'middle', 'latest'].map(id => backendMessage({ id, remote_uidvalidity: generation }));
  previewMock.mockImplementation(async () => previews() as never);
  bodiesMock.mockImplementation(async ids => ids.map(id => ({ ...inlineBody(''), id, body_text: `${id} body`, remote_uidvalidity: id === 'old' ? 8 : generation })) as never);
  const messages = useMessagesStore();
  await messages.openThread('th-live');
  let watchRuns = 0;
  const stop = watch(() => messages.getByThread('th-live').map(message => message.id).join('|'), () => {
    watchRuns++;
    void messages.loadAttachmentsForMessages('th-live', ['old']);
  }, { immediate: true });
  generation = 8;
  await nextTick();
  await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.bodyText).toBe('old body'));
  expect(messages.getByThread('th-live')[0]?.bodyLoaded).toBe(true);
  expect(messages.getByThread('th-live')[0]?.remoteUidValidity).toBe(8);
  expect(watchRuns).toBe(1);
  stop();
});

});

describe('bounded content revalidation', () => {
  it('stops refreshing an unverifiable response for an unchanged preview reference', async () => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    commandsAvailableMock.mockReturnValue(true);
    attachmentsMock.mockResolvedValue([]);
    previewMock.mockResolvedValue([backendMessage({ remote_uidvalidity: 7 })] as never);
    bodiesMock.mockResolvedValue([{ ...inlineBody(''), remote_uidvalidity: 8 }] as never);
    const messages = useMessagesStore();
    await messages.openThread('th-live');
    await vi.waitFor(() => expect(messages.getByThread('th-live')[0]?.bodyLoadError).toMatch(/sync.*folder/i));
    expect(previewMock).toHaveBeenCalledTimes(2);
    expect(messages.getByThread('th-live')[0]?.bodyLoaded).toBe(true);
  });
});


describe('invalid previews and interrupted body refreshes', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    commandsAvailableMock.mockReturnValue(true);
    previewMock.mockReset();
    bodiesMock.mockReset();
    attachmentsMock.mockReset().mockResolvedValue([]);
    loadBodyMock.mockReset();
  });

  it.each([null, 7])('never exposes invalid preview content with saved generation %s', async (generation) => {
    const content_error = JSON.stringify({ key: 'backend.errors.imapUidValidityChanged', params: { folder: 'INBOX' } });
    previewMock.mockResolvedValue([
      backendMessage({ id: 'old', remote_uidvalidity: generation, content_error, body_loaded: true,
        body_text: '', snippet: 'STALE CACHED CONTENT', body_html: '<p>STALE HTML</p>', source_payload: '{"text":"stale payload"}' }),
      backendMessage({ id: 'middle' }), backendMessage({ id: 'latest' }),
    ] as never);
    bodiesMock.mockImplementation(async ids => ids.map(id => ({ ...inlineBody('<p>safe</p>'), id })) as never);
    const store = useMessagesStore();
    await store.openThread('th-live');
    const message = store.getByThread('th-live')[0]!;
    expect(message.bodyText).toBe('');
    expect(message.bodyHtml).toBeUndefined();
    expect(message.sourcePayload).toBeUndefined();
    expect(message.bodyLoadError).toMatch(/sync.*folder/i);
    const { forwardThreadPrefill, replyThreadPrefill } = await import('@/utils/composerThreadPrefill');
    for (const prefill of [forwardThreadPrefill([message], 'Subject'), replyThreadPrefill('reply', [message], 'Subject', new Set<string>())]) {
      expect(prefill.body ?? '').not.toContain('STALE');
      expect(prefill.bodyHtml ?? '').not.toContain('STALE');
    }
  });

  it.each(['before', 'after', 'reject'])('replaces an obsolete body request whose completion arrives %s the new one', async (order) => {
    previewMock.mockResolvedValue(['old', 'middle', 'latest'].map(id => backendMessage({ id, remote_uidvalidity: 7 })) as never);
    bodiesMock.mockImplementation(async ids => ids.map(id => ({ ...inlineBody(''), id, remote_uidvalidity: 7,
      body_loaded: id !== 'old', body_text: id === 'old' ? '' : 'Cached latest' })) as never);
    type Loaded = Awaited<ReturnType<typeof backendLoadMessageBody>>;
    let resolveOld!: (body: Loaded) => void;
    let rejectOld!: (error: Error) => void;
    let resolveCurrent!: (body: Loaded) => void;
    loadBodyMock.mockImplementationOnce(() => new Promise((resolve, reject) => { resolveOld = resolve; rejectOld = reject; }));
    loadBodyMock.mockImplementationOnce(() => new Promise(resolve => { resolveCurrent = resolve; }));
    const store = useMessagesStore();
    await store.openThread('th-live');
    const expansion = store.loadAttachmentsForMessages('th-live', ['old']);
    await vi.waitFor(() => expect(loadBodyMock).toHaveBeenCalledTimes(1));
    const refresh = store.refreshThread('th-live');
    await vi.waitFor(() => expect(loadBodyMock).toHaveBeenCalledTimes(2));
    const oldBody = { body_text: 'Obsolete body', body_html: null, remote_uidvalidity: 7, fetched: true };
    if (order !== 'after') {
      if (order === 'reject') rejectOld(new Error('Obsolete failure'));
      else resolveOld(oldBody);
      await expansion;
      expect(store.isBodyLoading('old')).toBe(true);
      expect(store.getByThread('th-live')[0]?.bodyLoadError).toBeUndefined();
    }
    resolveCurrent({ ...oldBody, body_text: 'Current body' });
    await refresh;
    if (order === 'after') { resolveOld(oldBody); await expansion; }
    expect(store.isBodyLoading('old')).toBe(false);
    expect(store.getByThread('th-live')[0]?.bodyLoaded).toBe(true);
    expect(store.getByThread('th-live')[0]?.bodyText).toBe('Current body');
    expect(store.getByThread('th-live')[0]?.bodyLoadError).toBeUndefined();
  });
});

describe('inline-image requests interrupted by refresh', () => {
  it.each(['before', 'after', 'reject'])('retains expanded images when the obsolete download completes %s', async (order) => {
    const { watch, nextTick } = await import('vue');
    setActivePinia(createPinia());
    vi.clearAllMocks();
    downloadAttachmentMock.mockReset();
    commandsAvailableMock.mockReturnValue(true);
    previewMock.mockResolvedValue(['old', 'middle', 'latest'].map(id => backendMessage({ id, remote_uidvalidity: 7 })) as never);
    bodiesMock.mockImplementation(async ids => ids.map(id => ({
      ...inlineBody(id === 'old' ? '<img src="cid:image001.jpg@01DD16D4.C2220470">' : ''),
      id, remote_uidvalidity: 7,
    })) as never);
    attachmentsMock.mockImplementation(async ids => ids.includes('old')
      ? [inlineAttachmentRow({ message_id: 'old' })] as never : []);
    type Download = Awaited<ReturnType<typeof backendDownloadAttachment>>;
    const obsolete = deferred<Download>();
    const current = deferred<Download>();
    downloadAttachmentMock.mockReturnValueOnce(obsolete.promise).mockReturnValueOnce(current.promise);
    const store = useMessagesStore();
    await store.openThread('th-live');
    const expansion = store.loadAttachmentsForMessages('th-live', ['old']);
    await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalledTimes(1));
    // Match the visible-message predicate used by ThreadView for the expanded row.
    const stop = watch(() => store.getByThread('th-live').filter(message => message.id === 'old')
      .filter(message => (message.bodyLoaded === false && !store.isBodyLoading(message.id))
        || (message.bodyLoaded !== false && store.needsDetails(message.id)))
      .map(message => message.id).join('\u0000'), key => {
        if (key) void store.loadAttachmentsForMessages('th-live', key.split('\u0000'));
      }, { flush: 'post', immediate: true });
    const result = (data_base64: string): Download => ({
      filename: 'image.jpg', mime_type: 'image/jpeg', data_base64, remote_uidvalidity: 7,
    });
    try {
      await store.refreshThread('th-live');
      await vi.waitFor(() => expect(downloadAttachmentMock).toHaveBeenCalledTimes(2));
      expect(bodiesMock.mock.calls.filter(([ids]) => ids.includes('old'))).toHaveLength(1);
      if (order !== 'after') {
        if (order === 'reject') obsolete.reject(new Error('obsolete download failed'));
        else obsolete.resolve(result('b2xk'));
        await expansion;
        await nextTick();
        expect(store.needsDetails('old')).toBe(false);
        expect(store.getByThread('th-live')[0]?.attachments).toEqual([]);
        expect(store.getByThread('th-live')[0]?.inlineImagesError).toBeUndefined();
      }
      current.resolve(result('bmV3'));
      await vi.waitFor(() => expect(store.getByThread('th-live')[0]?.attachments[0]?.dataUrl)
        .toBe('data:image/jpeg;base64,bmV3'));
      if (order === 'after') {
        obsolete.resolve(result('b2xk'));
        await expansion;
        await nextTick();
      }
      expect(store.getByThread('th-live')[0]?.attachments[0]?.dataUrl).toBe('data:image/jpeg;base64,bmV3');
      expect(store.getByThread('th-live')[0]?.inlineImagesError).toBeUndefined();
      expect(store.needsDetails('old')).toBe(false);
      expect(downloadAttachmentMock).toHaveBeenCalledTimes(2);
    } finally {
      stop();
    }
  });
});
