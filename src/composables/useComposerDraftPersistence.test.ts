// @vitest-environment jsdom

import { defineComponent, h, nextTick, ref } from 'vue';
import { mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendDraft, BackendDraftInput, BackendDraftSendOptions } from '@/types/composer';
import {
  flushOpenComposerDrafts,
  useComposerDraftPersistence,
  type ComposerStatus,
} from './useComposerDraftPersistence';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function savedDraft(input: BackendDraftInput, overrides: Partial<BackendDraft> = {}): BackendDraft {
  return {
    id: input.id ?? 'saved-draft',
    account_id: input.accountId,
    thread_id: input.threadId ?? null,
    to: input.to ?? [],
    cc: input.cc ?? [],
    bcc: input.bcc ?? [],
    subject: input.subject ?? '',
    body: input.body ?? '',
    body_html: input.bodyHtml ?? null,
    attachments: input.attachments ?? [],
    updated_at: '2026-08-31T10:00:00.000Z',
    ...overrides,
  };
}

interface HarnessOptions {
  save?: (input: BackendDraftInput) => Promise<BackendDraft | null>;
  remove?: (id: string) => Promise<void>;
  initialized?: boolean;
  intervalMs?: number;
  onCaughtUp?: () => void;
  onSaved?: (input: Omit<BackendDraftInput, 'id' | 'expectedRevision'>) => void;
}

function createHarness(options: HarnessOptions = {}) {
  const initialized = ref(options.initialized ?? true);
  const sending = ref(false);
  const status = ref<ComposerStatus | null>(null);
  const to = ref<string[]>([]);
  const cc = ref<string[]>([]);
  const bcc = ref<string[]>([]);
  const sendOptions = ref<BackendDraftSendOptions | null>(null);
  const subject = ref('');
  const body = ref('');
  const bodyHtml = ref<string | null>(null);
  const attachments = ref<BackendDraftInput['attachments']>([]);
  const save = vi.fn(options.save ?? (async (input) => savedDraft(input)));
  const remove = vi.fn(options.remove ?? (async () => undefined));
  const buildInput = vi.fn((): Omit<BackendDraftInput, 'id' | 'expectedRevision'> => ({
    ...(sendOptions.value ? { sendOptions: sendOptions.value } : {}),
    accountId: 'account-1',
    threadId: 'thread-1',
    to: to.value,
    cc: cc.value,
    bcc: bcc.value,
    subject: subject.value,
    body: body.value,
    bodyHtml: bodyHtml.value,
    fromAddress: 'sender@example.com',
    fromName: 'Sender',
    replyTo: 'reply@example.com',
    scheduledFor: '2026-09-01T09:30:00.000Z',
    attachments: attachments.value,
    signatureId: 'signature-1',
    signatureVariant: 'long',
    signatureOverrideText: 'Regards',
    signatureOverrideHtml: '<p>Regards</p>',
  }));
  const translate = vi.fn((key: string, params?: Record<string, string | number>) =>
    params?.error ? `${key}:${params.error}` : key,
  );
  let persistence!: ReturnType<typeof useComposerDraftPersistence>;
  const wrapper = mount(defineComponent({
    setup() {
      persistence = useComposerDraftPersistence({
        initialized,
        sending,
        status,
        buildInput,
        drafts: { save, remove },
        t: translate,
        autosaveIntervalMs: options.intervalMs ?? 1_000,
        onCaughtUp: options.onCaughtUp,
        onSaved: options.onSaved,
      });
      return () => h('div');
    },
  }));

  return {
    wrapper,
    initialized,
    sending,
    status,
    to,
    cc,
    bcc,
    subject,
    sendOptions,
    body,
    bodyHtml,
    attachments,
    save,
    remove,
    buildInput,
    translate,
    persistence,
  };
}

const wrappers: VueWrapper[] = [];

function harness(options: HarnessOptions = {}) {
  const result = createHarness(options);
  wrappers.push(result.wrapper);
  return result;
}

describe('useComposerDraftPersistence', () => {
  beforeEach(() => {
    vi.stubGlobal('crypto', { randomUUID: vi.fn(() => 'stable-id') });
  });

  afterEach(() => {
    while (wrappers.length > 0) wrappers.pop()?.unmount();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('skips an empty new draft but persists an existing draft that was emptied', async () => {
    const { persistence, save } = harness();

    await expect(persistence.persist()).resolves.toBeNull();
    expect(save).not.toHaveBeenCalled();
    expect(persistence.draftId.value).toBeNull();

    persistence.draftId.value = 'existing-draft';
    await expect(persistence.persist()).resolves.toBe('existing-draft');
    expect(save).toHaveBeenCalledWith(expect.objectContaining({
      id: 'existing-draft',
      subject: '',
      body: '',
    }));
  });

  it.each(['cc', 'bcc'] as const)('persists a new draft whose only content is %s', async field => {
    const editor = harness();
    editor[field].value = ['copy@example.com'];
    await nextTick();
    await expect(editor.persistence.persist()).resolves.toBe('draft-stable-id');
    expect(editor.save).toHaveBeenCalledWith(expect.objectContaining({ [field]: ['copy@example.com'] }));
  });

  it('does not rebuild the draft input on every keystroke', async () => {
    vi.useFakeTimers();
    const editor = harness();
    editor.buildInput.mockClear();

    for (const text of ['H', 'He', 'Hel', 'Hell', 'Hello']) {
      editor.body.value = text;
      await nextTick();
      await vi.advanceTimersByTimeAsync(50);
    }
    expect(editor.buildInput).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(300);
    expect(editor.buildInput).toHaveBeenCalledTimes(1);
    expect(editor.persistence.dirty.value).toBe(true);
  });

  it('flushes the latest keystroke on quit even before typing pauses', async () => {
    vi.useFakeTimers();
    const editor = harness();
    editor.body.value = 'Hello world';
    await nextTick();

    await expect(flushOpenComposerDrafts()).resolves.toBe(true);

    expect(editor.save).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ body: 'Hello world' }));
  });

  it('reports exactly what each successful save put on disk', async () => {
    const pending = deferred<BackendDraft | null>();
    const onSaved = vi.fn();
    const editor = harness({ save: async () => pending.promise, onSaved });
    editor.body.value = 'Hello world';
    await nextTick();

    const saving = editor.persistence.persist();
    // Typed while the save is in flight: not part of what was saved.
    editor.body.value = 'Hello';
    pending.resolve(null);
    await saving;

    expect(onSaved).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ body: 'Hello world' }));
    expect(editor.persistence.dirty.value).toBe(true);
  });

  it('autosaves a policy-only edit after the text is already durable', async () => {
    vi.useFakeTimers();
    const editor = harness({ intervalMs: 100 });
    editor.body.value = 'Private message';
    await nextTick();
    await editor.persistence.persist();
    editor.persistence.startAutosave();
    editor.sendOptions.value = { pgp_encrypt_to_recipients: true, priority: 'high' };
    await nextTick();
    expect(editor.persistence.hasUnsavedChanges()).toBe(true);
    await vi.advanceTimersByTimeAsync(100);
    expect(editor.save).toHaveBeenLastCalledWith(expect.objectContaining({
      sendOptions: { pgp_encrypt_to_recipients: true, priority: 'high' },
    }));
    expect(editor.persistence.dirty.value).toBe(false);
  });

  it('publishes one draft id before saving and forwards every draft field', async () => {
    const pending = deferred<BackendDraft | null>();
    const { persistence, save, to, subject, body, bodyHtml, attachments } = harness({
      save: async () => pending.promise,
    });
    to.value = ['reader@example.com'];
    subject.value = 'Subject';
    body.value = 'Body';
    bodyHtml.value = '<p>Body</p>';
    attachments.value = [{
      filename: 'notes.txt',
      mime_type: 'text/plain',
      content_base64: 'Tm90ZXM=',
      inline: false,
    }];
    await nextTick();

    const saving = persistence.persist();

    expect(persistence.draftId.value).toBe('draft-stable-id');
    expect(save).toHaveBeenCalledWith({
      id: 'draft-stable-id',
      accountId: 'account-1',
      threadId: 'thread-1',
      to: ['reader@example.com'],
      cc: [],
      bcc: [],
      subject: 'Subject',
      body: 'Body',
      bodyHtml: '<p>Body</p>',
      fromAddress: 'sender@example.com',
      fromName: 'Sender',
      replyTo: 'reply@example.com',
      scheduledFor: '2026-09-01T09:30:00.000Z',
      attachments: [{
        filename: 'notes.txt',
        mime_type: 'text/plain',
        content_base64: 'Tm90ZXM=',
        inline: false,
      }],
      signatureId: 'signature-1',
      signatureVariant: 'long',
      signatureOverrideText: 'Regards',
      signatureOverrideHtml: '<p>Regards</p>',
      expectedRevision: null,
    });

    pending.resolve(savedDraft(save.mock.calls[0]![0]));
    await expect(saving).resolves.toBe('draft-stable-id');
  });

  it('keeps edits made during a save dirty and coalesces queued saves without overlap', async () => {
    const first = deferred<BackendDraft | null>();
    let activeSaves = 0;
    let maxActiveSaves = 0;
    const save = vi.fn(async (input: BackendDraftInput) => {
      activeSaves += 1;
      maxActiveSaves = Math.max(maxActiveSaves, activeSaves);
      const result = save.mock.calls.length === 1
        ? await first.promise
        : savedDraft(input);
      activeSaves -= 1;
      return result;
    });
    const { persistence, body } = harness({ save });
    body.value = 'first revision';
    await nextTick();

    const initial = persistence.persist();
    body.value = 'second revision';
    await nextTick();
    const queued = persistence.persist();

    expect(save).toHaveBeenCalledTimes(1);
    expect(persistence.dirty.value).toBe(true);

    first.resolve(savedDraft(save.mock.calls[0]![0]));
    await expect(Promise.all([initial, queued])).resolves.toEqual([
      'draft-stable-id',
      'draft-stable-id',
    ]);
    expect(save).toHaveBeenCalledTimes(2);
    expect(save.mock.calls[1]![0].body).toBe('second revision');
    expect(maxActiveSaves).toBe(1);
    expect(persistence.dirty.value).toBe(false);
  });

  it('leaves an edit dirty when no caller queues a follow-up save', async () => {
    const first = deferred<BackendDraft | null>();
    const { persistence, save, body } = harness({ save: async () => first.promise });
    body.value = 'first revision';
    await nextTick();
    const saving = persistence.persist();

    body.value = 'second revision';
    await nextTick();
    first.resolve(savedDraft(save.mock.calls[0]![0]));
    await saving;

    expect(save).toHaveBeenCalledTimes(1);
    expect(persistence.dirty.value).toBe(true);
  });

  it('cancels a queued save, waits for the active save, and stays paused until resumed', async () => {
    const first = deferred<BackendDraft | null>();
    const { persistence, save, body } = harness({
      save: vi.fn()
        .mockImplementationOnce(async () => first.promise)
        .mockImplementation(async (input: BackendDraftInput) => savedDraft(input)),
    });
    body.value = 'first revision';
    await nextTick();
    const saving = persistence.persist();
    body.value = 'second revision';
    await nextTick();
    void persistence.persist();
    const pausing = persistence.pause();

    expect(save).toHaveBeenCalledTimes(1);
    first.resolve(savedDraft(save.mock.calls[0]![0]));
    await Promise.all([saving, pausing]);
    expect(save).toHaveBeenCalledTimes(1);
    await expect(persistence.persist()).resolves.toBeNull();

    persistence.resume();
    await persistence.persist();
    expect(save).toHaveBeenCalledTimes(2);
    expect(save.mock.calls[1]![0].body).toBe('second revision');
  });

  it('distinguishes a locally saved remote failure from a local save failure', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const { persistence, save, body, status } = harness({
      save: vi.fn()
        .mockImplementationOnce(async (input: BackendDraftInput) => savedDraft(input, {
          remote_error: ' provider unavailable ',
        }))
        .mockRejectedValueOnce(new Error('disk full'))
        .mockImplementation(async (input: BackendDraftInput) => savedDraft(input)),
    });
    body.value = 'draft';
    await nextTick();

    await persistence.persist({ silent: true });
    expect(status.value).toEqual({
      tone: 'warning',
      text: 'compose.status.draftSavedRemotePending:provider unavailable',
    });

    body.value = 'new draft';
    await nextTick();
    status.value = null;
    await persistence.persist({ silent: true });
    expect(status.value).toBeNull();
    expect(persistence.dirty.value).toBe(true);

    await persistence.persist();
    expect(save).toHaveBeenCalledTimes(3);
    expect(status.value).toEqual({ tone: 'info', text: 'compose.status.draftSaved' });
    expect(warn).toHaveBeenCalledWith('draft save failed', expect.any(Error));
  });

  it('runs autosave only for dirty, active drafts and stops it after finishing', async () => {
    vi.useFakeTimers();
    const { persistence, save, body, sending } = harness({ intervalMs: 100 });
    persistence.startAutosave();
    persistence.startAutosave();

    await vi.advanceTimersByTimeAsync(100);
    expect(save).not.toHaveBeenCalled();

    body.value = 'autosave me';
    await nextTick();
    sending.value = true;
    await vi.advanceTimersByTimeAsync(100);
    expect(save).not.toHaveBeenCalled();

    sending.value = false;
    await vi.advanceTimersByTimeAsync(100);
    expect(save).toHaveBeenCalledTimes(1);
    expect(persistence.dirty.value).toBe(false);

    body.value = 'do not save after finish';
    await nextTick();
    persistence.finish();
    await vi.advanceTimersByTimeAsync(200);
    expect(save).toHaveBeenCalledTimes(1);
  });

  it('settles active work before deleting and restarts autosave after a failed discard', async () => {
    vi.useFakeTimers();
    const active = deferred<BackendDraft | null>();
    const remove = vi.fn()
      .mockRejectedValueOnce(new Error('delete failed'))
      .mockResolvedValue(undefined);
    const { persistence, save, body } = harness({
      intervalMs: 100,
      save: vi.fn()
        .mockImplementationOnce(async () => active.promise)
        .mockImplementation(async (input: BackendDraftInput) => savedDraft(input)),
      remove,
    });
    persistence.draftId.value = 'draft-1';
    body.value = 'draft';
    await nextTick();
    const saving = persistence.persist();
    const discarding = persistence.discard({ resumeAutosave: true });
    expect(remove).not.toHaveBeenCalled();

    active.resolve(savedDraft(save.mock.calls[0]![0]));
    await saving;
    await expect(discarding).rejects.toThrow('delete failed');
    expect(remove).toHaveBeenCalledWith('draft-1');
    expect(persistence.draftId.value).toBe('draft-1');

    body.value = 'changed after failure';
    await nextTick();
    await vi.advanceTimersByTimeAsync(100);
    expect(save).toHaveBeenCalledTimes(2);

    await persistence.discard({ resumeAutosave: true });
    expect(persistence.draftId.value).toBeNull();
  });

  it('saves a dirty draft on unmount unless sending or explicitly paused', async () => {
    const first = harness();
    first.body.value = 'save on close';
    await nextTick();
    first.wrapper.unmount();
    wrappers.splice(wrappers.indexOf(first.wrapper), 1);
    await Promise.resolve();
    expect(first.save).toHaveBeenCalledTimes(1);

    const sending = harness();
    sending.body.value = 'already sending';
    sending.sending.value = true;
    await nextTick();
    sending.wrapper.unmount();
    wrappers.splice(wrappers.indexOf(sending.wrapper), 1);
    await Promise.resolve();
    expect(sending.save).not.toHaveBeenCalled();

    const paused = harness();
    paused.body.value = 'discarding';
    await nextTick();
    await paused.persistence.pause();
    paused.wrapper.unmount();
    wrappers.splice(wrappers.indexOf(paused.wrapper), 1);
    await Promise.resolve();
    expect(paused.save).not.toHaveBeenCalled();
  });

  /** Autosave errors used to be swallowed; the editor kept looking saved. */
  it('keeps a failed save visible until a save lands', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const onCaughtUp = vi.fn();
    const { persistence, body } = harness({
      onCaughtUp,
      save: vi.fn()
        .mockRejectedValueOnce(new Error('disk full'))
        .mockImplementation(async (input: BackendDraftInput) => savedDraft(input)),
    });
    body.value = 'ten minutes of typing';
    await nextTick();

    await persistence.persist({ silent: true });
    expect(persistence.saveError.value).toEqual({ kind: 'failed', message: 'disk full' });
    expect(onCaughtUp).not.toHaveBeenCalled();

    await persistence.persist({ silent: true });
    expect(persistence.saveError.value).toBeNull();
    expect(onCaughtUp).toHaveBeenCalledTimes(1);
  });

  it('sends the last confirmed revision and stops autosaving into a conflict', async () => {
    vi.useFakeTimers();
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const save = vi.fn()
      .mockImplementationOnce(async (input: BackendDraftInput) => savedDraft(input, { revision: 4 }))
      .mockRejectedValueOnce({ key: 'backend.errors.draftChangedElsewhere' })
      .mockImplementation(async (input: BackendDraftInput) => savedDraft(input, { revision: 1 }));
    const { persistence, body } = harness({ save, intervalMs: 100 });
    body.value = 'first';
    await nextTick();
    await persistence.persist();
    expect(persistence.serverRevision.value).toBe(4);

    body.value = 'second';
    await nextTick();
    await persistence.persist();
    expect(save.mock.calls[1]![0].expectedRevision).toBe(4);
    expect(persistence.saveError.value?.kind).toBe('conflict');

    persistence.startAutosave();
    await vi.advanceTimersByTimeAsync(300);
    expect(save).toHaveBeenCalledTimes(2);

    vi.stubGlobal('crypto', { randomUUID: vi.fn(() => 'fresh-id') });
    await expect(persistence.saveAsNewDraft()).resolves.toBe('draft-fresh-id');
    expect(save.mock.calls[2]![0]).toMatchObject({ id: 'draft-fresh-id', expectedRevision: null });
    expect(persistence.saveError.value).toBeNull();
  });

  it('does not mark setup-time changes dirty before initialization', async () => {
    const { initialized, body, persistence } = harness({ initialized: false });
    body.value = 'loaded draft';
    await nextTick();
    expect(persistence.hasUnsavedChanges()).toBe(false);

    initialized.value = true;
    body.value = 'edited draft';
    await nextTick();
    expect(persistence.hasUnsavedChanges()).toBe(true);
  });
});

/**
 * Quitting used to end the process between two autosave ticks, and up to ten
 * seconds of typing went with it. The backend asks first now.
 */
describe('flushing before the app exits', () => {
  const wrappers: VueWrapper[] = [];

  afterEach(() => {
    while (wrappers.length > 0) wrappers.pop()?.unmount();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  function open() {
    vi.stubGlobal('crypto', { randomUUID: vi.fn(() => 'stable-id') });
    const result = createHarness();
    wrappers.push(result.wrapper);
    return result;
  }

  it('writes a dirty draft once, whether or not the timer was due', async () => {
    const composer = open();
    composer.subject.value = 'Halbfertig';
    await nextTick();

    await flushOpenComposerDrafts();

    expect(composer.save).toHaveBeenCalledTimes(1);
    expect(composer.save.mock.calls[0]?.[0].subject).toBe('Halbfertig');
    // Silent: a quit is no place for a "draft saved" line nobody reads.
    expect(composer.status.value).toBeNull();
  });

  it('answers without writing when there is nothing unsaved', async () => {
    const composer = open();

    await expect(flushOpenComposerDrafts()).resolves.toBe(true);

    expect(composer.save).not.toHaveBeenCalled();
  });

  /** ⌘Q used to exit even when this save failed. */
  it('answers false when a draft could not be written', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const composer = open();
    composer.save.mockRejectedValue(new Error('disk full'));
    composer.subject.value = 'Halbfertig';
    await nextTick();

    await expect(flushOpenComposerDrafts()).resolves.toBe(false);
    // Still unsaved on the next attempt, even though nothing changed since.
    await expect(flushOpenComposerDrafts()).resolves.toBe(false);
  });

  it('reaches every open composer and forgets the closed ones', async () => {
    const first = open();
    const second = open();
    first.subject.value = 'Eins';
    second.subject.value = 'Zwei';
    await nextTick();

    await flushOpenComposerDrafts();
    expect(first.save).toHaveBeenCalledTimes(1);
    expect(second.save).toHaveBeenCalledTimes(1);

    second.wrapper.unmount();
    wrappers.splice(wrappers.indexOf(second.wrapper), 1);
    first.subject.value = 'Eins, weiter';
    await nextTick();

    await flushOpenComposerDrafts();
    expect(first.save).toHaveBeenCalledTimes(2);
    expect(second.save).toHaveBeenCalledTimes(1);
  });
});
