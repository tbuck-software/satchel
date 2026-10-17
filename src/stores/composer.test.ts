import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useComposerStore } from './composer';

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
    const guard = vi.fn(async () => false);
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
      return true;
    });

    await expect(composer.openEmail(['mara@example.com'])).resolves.toBe(true);

    expect(composer.request?.to).toEqual(['mara@example.com']);
  });

  it('drops a request that a newer one overtook while the host was asking', async () => {
    const composer = useComposerStore();
    await composer.openNew();
    let answer!: (value: boolean) => void;
    composer.setReplaceGuard(() => new Promise<boolean>((resolve) => { answer = resolve; }));

    const first = composer.openReply('thread-1');
    const pendingAnswer = answer;
    const second = composer.openForward('thread-2');
    pendingAnswer(true);
    answer(true);

    await expect(first).resolves.toBe(false);
    await expect(second).resolves.toBe(true);
    expect(composer.request).toMatchObject({ mode: 'forward', threadId: 'thread-2' });
  });

  it('opens without asking when nothing is open', async () => {
    const composer = useComposerStore();
    const guard = vi.fn(async () => false);
    composer.setReplaceGuard(guard);

    await expect(composer.openNew()).resolves.toBe(true);
    expect(guard).not.toHaveBeenCalled();
  });
});
