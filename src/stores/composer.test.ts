import { beforeEach, describe, expect, it, onTestFinished, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { REPLACE_GUARD_WAIT_MS, useComposerStore, type ComposerReplaceOutcome } from './composer';

describe('composer store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('opens a new message with mailto fields prefilled', () => {
    const composer = useComposerStore();

    composer.openEmail(['mara@example.com'], {
      subject: 'Hallo',
      body: 'Guten Tag',
    });

    expect(composer.request).toEqual({
      mode: 'new',
      threadId: null,
      draftId: null,
      to: ['mara@example.com'],
      cc: [],
      bcc: [],
      subject: 'Hallo',
      body: 'Guten Tag',
      bodyHtml: null,
    });
  });

  it('carries a rescued outbox message with its copies and its HTML body', () => {
    const composer = useComposerStore();

    composer.open({
      mode: 'new',
      to: ['a@x.de'],
      cc: ['c@x.de'],
      bcc: ['b@x.de'],
      subject: 'Angebot',
      body: 'Guten Tag,',
      bodyHtml: '<p>Guten Tag,</p>',
    });

    expect(composer.request).toMatchObject({
      cc: ['c@x.de'],
      bcc: ['b@x.de'],
      bodyHtml: '<p>Guten Tag,</p>',
    });
  });
});

describe('replacing an open composer', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('asks the host before a second request replaces the open composer', async () => {
    const composer = useComposerStore();
    await composer.openNew();
    const firstInstance = composer.instance;
    const guard = vi.fn(async () => 'kept' as const);
    composer.setReplaceGuard(guard);

    await expect(composer.openReply('thread-1')).resolves.toBe(false);

    expect(guard).toHaveBeenCalledWith(expect.objectContaining({ mode: 'reply', threadId: 'thread-1' }));
    expect(composer.request?.mode).toBe('new');
    expect(composer.instance).toBe(firstInstance);
  });

  it('opens the new request once the host has closed the old one', async () => {
    const composer = useComposerStore();
    await composer.openNew();
    composer.setReplaceGuard(async () => {
      composer.close();
      return 'replaced';
    });

    await expect(composer.openEmail(['mara@example.com'])).resolves.toBe(true);

    expect(composer.request?.to).toEqual(['mara@example.com']);
  });

  it('decides one request at a time, so a second one never takes the first one\'s question away', async () => {
    const composer = useComposerStore();
    await composer.openNew();
    const answers: ((value: ComposerReplaceOutcome) => void)[] = [];
    const guard = vi.fn(() => new Promise<ComposerReplaceOutcome>((resolve) => {
      answers.push((value) => {
        if (value === 'replaced') composer.close();
        resolve(value);
      });
    }));
    composer.setReplaceGuard(guard);

    const first = composer.openWithOutcome({ mode: 'reply', threadId: 'thread-1', body: 'first reply' });
    const second = composer.openWithOutcome({ mode: 'forward', threadId: 'thread-2' });
    await Promise.resolve();
    await Promise.resolve();
    expect(guard).toHaveBeenCalledOnce();

    answers[0]!('replaced');
    await expect(first).resolves.toBe('opened');
    expect(composer.request).toMatchObject({ mode: 'reply', threadId: 'thread-1' });
    // The second request meets the composer the first decision left open.
    await vi.waitFor(() => expect(guard).toHaveBeenCalledTimes(2));
    expect(guard).toHaveBeenLastCalledWith(expect.objectContaining({ mode: 'forward', threadId: 'thread-2' }));
    answers[1]!('kept');
    await expect(second).resolves.toBe('kept');
    expect(composer.request).toMatchObject({ mode: 'reply', body: 'first reply' });
  });

  it('keeps taking requests after a host guard throws', async () => {
    const composer = useComposerStore();
    await composer.openNew();
    composer.setReplaceGuard(vi.fn<() => Promise<ComposerReplaceOutcome>>()
      .mockRejectedValueOnce(new Error('guard failed'))
      .mockImplementation(async () => {
        composer.close();
        return 'replaced';
      }));

    const failed = composer.openWithOutcome({ mode: 'reply', threadId: 'thread-1' });
    const next = composer.openWithOutcome({ mode: 'forward', threadId: 'thread-2' });
    await expect(failed).rejects.toThrow('guard failed');
    await expect(next).resolves.toBe('opened');
  });

  it('carries the mark of a body that exists nowhere else, and only when set', async () => {
    const composer = useComposerStore();
    await composer.open({ mode: 'reply', threadId: 'thread-1', body: 'typed in a notification', unsavedBody: true });
    expect(composer.request).toMatchObject({ body: 'typed in a notification', unsavedBody: true });
    composer.close();
    await composer.openReply('thread-1');
    expect(composer.request).not.toHaveProperty('unsavedBody');
  });

  it.each(['kept', 'unsaved', 'interrupted'] as const)('tells a caller that the open composer answered %s', async (outcome) => {
    const composer = useComposerStore();
    await composer.open({ mode: 'new', body: 'open draft' });
    composer.setReplaceGuard(async () => outcome);

    await expect(composer.openWithOutcome({ mode: 'reply', threadId: 'thread-1', body: 'reply' }))
      .resolves.toBe(outcome);
    expect(composer.request?.body).toBe('open draft');
  });

  it('opens without asking when nothing is open', async () => {
    const composer = useComposerStore();
    const guard = vi.fn(async () => 'kept' as const);
    composer.setReplaceGuard(guard);

    await expect(composer.openNew()).resolves.toBe(true);
    expect(guard).not.toHaveBeenCalled();
  });

  // App.vue loads the host as an async chunk behind `composer.isOpen`, so its
  // guard registers only after the first open. A second request in that gap
  // used to replace text that existed nowhere else without a question.
  it('waits for the host to register its guard before replacing a body that exists nowhere else', async () => {
    const composer = useComposerStore();
    await expect(composer.openWithOutcome({ mode: 'reply', threadId: 'a', body: 'reply text A', unsavedBody: true }))
      .resolves.toBe('opened');

    const second = composer.openWithOutcome({ mode: 'reply', threadId: 'b', body: 'reply text B', unsavedBody: true });
    await new Promise(resolve => setTimeout(resolve, 20));
    expect(composer.request?.body).toBe('reply text A');

    const guard = vi.fn(async () => 'kept' as const);
    composer.setReplaceGuard(guard);
    await expect(second).resolves.toBe('kept');
    expect(guard).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ threadId: 'b' }));
    expect(composer.request?.body).toBe('reply text A');
  });

  it('leaves such a body open, unanswered, when no host comes to ask', async () => {
    vi.useFakeTimers();
    onTestFinished(() => { vi.useRealTimers(); });
    const composer = useComposerStore();
    await composer.open({ mode: 'reply', threadId: 'a', body: 'reply text A', unsavedBody: true });

    const second = composer.openWithOutcome({ mode: 'reply', threadId: 'b', body: 'reply text B' });
    await vi.advanceTimersByTimeAsync(REPLACE_GUARD_WAIT_MS);
    await expect(second).resolves.toBe('interrupted');
    expect(composer.request?.body).toBe('reply text A');
    // The queue moves on, and a guard that registers late is used.
    composer.setReplaceGuard(async () => 'kept');
    await expect(composer.openWithOutcome({ mode: 'new' })).resolves.toBe('kept');
  });

  it('opens at once when that composer closes while the request waits for a host', async () => {
    const composer = useComposerStore();
    await composer.open({ mode: 'reply', threadId: 'a', body: 'reply text A', unsavedBody: true });
    const second = composer.openWithOutcome({ mode: 'reply', threadId: 'b', body: 'reply text B' });
    await Promise.resolve();

    composer.close();
    await expect(second).resolves.toBe('opened');
    expect(composer.request?.body).toBe('reply text B');
  });

  it('does not wait for a host to replace a composer that holds nothing of its own', async () => {
    const composer = useComposerStore();
    await composer.openReply('thread-1');

    await expect(composer.openWithOutcome({ mode: 'forward', threadId: 'thread-2' })).resolves.toBe('opened');
    expect(composer.request).toMatchObject({ mode: 'forward', threadId: 'thread-2' });
  });
});

describe('resuming a draft', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('heads a draft that turns out to be a reply as one', async () => {
    const composer = useComposerStore();
    await composer.openDraft('draft-1');
    expect(composer.title).toBe('New message');

    composer.adoptDraftMode('reply');

    expect(composer.request?.mode).toBe('reply');
    expect(composer.title).toBe('Reply');
  });

  it('leaves new messages and requests that already know their mode alone', async () => {
    const composer = useComposerStore();
    await composer.openNew();
    composer.adoptDraftMode('reply');
    expect(composer.request?.mode).toBe('new');

    await composer.open({ mode: 'forward', draftId: 'draft-2' });
    composer.adoptDraftMode('reply');
    expect(composer.request?.mode).toBe('forward');

    await composer.openDraft('draft-3');
    composer.adoptDraftMode('new');
    expect(composer.request?.mode).toBe('new');
  });
});
