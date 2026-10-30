// @vitest-environment jsdom
import { defineComponent, h, ref } from 'vue';
import { createPinia } from 'pinia';
import { EditorContent } from '@tiptap/vue-3';
import type { Editor } from '@tiptap/vue-3';
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendSendMessageResult } from '@/types/composer';
import type { Message } from '@/types/mail';
import type { ComposerDraftPersistenceOptions } from '@/composables/useComposerDraftPersistence';

const harness = vi.hoisted(() => ({
  sendMessage: vi.fn(),
  fetchDraft: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  resume: vi.fn(),
  finish: vi.fn(),
  push: vi.fn(),
  templatesLoad: vi.fn(),
  templatesLoadFailed: false,
  startAutosave: vi.fn(),
  captureBaseline: vi.fn(),
  isPopout: vi.fn(() => false),
  closePopout: vi.fn(async () => undefined),
  undoSend: vi.fn(),
  getByThread: vi.fn<() => Message[]>(() => []),
  loadAttachmentsForMessages: vi.fn(),
  composePlainText: true,
  realEditor: false,
  draftSetup: vi.fn<(options: ComposerDraftPersistenceOptions) => void>(),
}));

vi.mock('vue-router', () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => ({ push: harness.push }),
}));

vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => ({
    load: vi.fn().mockResolvedValue(undefined),
    activeAccountId: 'account-1',
    accounts: [{ id: 'account-1', email: 'me@example.com', displayName: 'Me' }],
    emailAccounts: [{ id: 'account-1', email: 'me@example.com', displayName: 'Me' }],
  }),
}));
vi.mock('@/stores/aliases', () => ({
  useAliasesStore: () => ({
    load: vi.fn().mockResolvedValue(undefined),
    items: [],
    aliasesFor: () => [],
  }),
}));
vi.mock('@/stores/drafts', () => ({
  useDraftsStore: () => ({ save: vi.fn(), remove: vi.fn(), fetchOne: harness.fetchDraft }),
}));
vi.mock('@/stores/messages', () => ({
  bodyHtmlWithInlineImages: (message: { bodyHtml?: string }) => message.bodyHtml,
  useMessagesStore: () => ({
    loadByThread: vi.fn().mockResolvedValue(undefined),
    getByThread: harness.getByThread,
    loadAttachmentsForMessages: harness.loadAttachmentsForMessages,
  }),
}));
vi.mock('@/stores/settings', () => ({
  useSettingsStore: () => ({ composePlainText: harness.composePlainText }),
}));
vi.mock('@/stores/templates', () => ({
  useTemplatesStore: () => ({ load: harness.templatesLoad, get loadFailed() { return harness.templatesLoadFailed; }, items: [] }),
}));
vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => ({ threads: [], load: vi.fn().mockResolvedValue(undefined) }),
}));

vi.mock('@/composables/useAttachmentActions', () => ({
  useAttachmentActions: () => ({ downloadAttachment: vi.fn() }),
}));
vi.mock('@/composables/useComposerCloseGuard', () => ({
  useComposerCloseGuard: () => ({
    captureBaseline: harness.captureBaseline,
    markSaved: vi.fn(),
    hasMeaningfulContent: () => true,
    shouldConfirmClose: () => false,
    closeWithoutSaving: vi.fn(),
    installStandaloneCloseGuard: vi.fn().mockResolvedValue(undefined),
    isPopout: harness.isPopout,
    closePopout: harness.closePopout,
  }),
}));
vi.mock('@/composables/useComposerDraftPersistence', () => ({
  useComposerDraftPersistence: (options: ComposerDraftPersistenceOptions) => {
    harness.draftSetup(options);
    return {
      draftId: ref<string | null>('draft-1'),
      serverRevision: ref<number | null>(null),
      saveError: ref(null),
      saveAsNewDraft: vi.fn().mockResolvedValue(null),
      persist: vi.fn().mockResolvedValue(null),
      startAutosave: harness.startAutosave,
      pause: vi.fn().mockResolvedValue(undefined),
      resume: harness.resume,
      finish: harness.finish,
      discard: vi.fn().mockResolvedValue(undefined),
      closeWithoutSaving: vi.fn(),
    };
  },
}));
vi.mock('@/composables/useComposerCrypto', () => ({
  useComposerCrypto: () => ({
    load: vi.fn().mockResolvedValue(undefined),
    smimeSignCertId: ref<string | null>(null),
    smimeEncrypt: ref(false),
    pgpSignKeyId: ref<string | null>(null),
    pgpEncrypt: ref(false),
    cryptoMethod: ref('none'),
    cryptoOptions: ref([]),
    canSignSelected: ref(false),
    canEncryptSelected: ref(false),
    cryptoSign: ref(false),
    cryptoEncrypt: ref(false),
    cryptoHint: ref(''),
  }),
}));
vi.mock('@/composables/useComposerInlineAttachments', () => ({
  useComposerInlineAttachments: () => ({
    hydrateInlineImages: (value: string | null) => value,
    serializeInlineImages: (value: string | null) => value,
    onInlineImageAdded: vi.fn(),
    onImageToAttachment: vi.fn(),
    addPastedFiles: vi.fn(),
    onAttachmentInsertedInline: vi.fn(),
    onInlineAttachmentRemoved: vi.fn(),
  }),
}));
vi.mock('@/composables/useComposerSignature', () => ({
  useComposerSignature: () => ({
    load: vi.fn().mockResolvedValue(undefined),
    availableSignatures: ref([]),
    selectedSignatureId: ref<string | null>(null),
    signatureVariant: ref('full'),
    signaturePreviewLoading: ref(false),
    signatureOverrideText: ref<string | null>(null),
    signatureOverrideHtml: ref<string | null>(null),
    signatureDisplayHtml: ref<string | null>(null),
    signatureDisplayText: ref<string | null>(null),
    signatureOverrideActive: ref(false),
    clearOverride: vi.fn(),
    syncSelection: vi.fn(),
    refreshPreview: vi.fn().mockResolvedValue(undefined),
    select: vi.fn(),
    selectVariant: vi.fn(),
    edit: vi.fn(),
  }),
}));
vi.mock('@/composables/useComposerTransport', () => ({
  useComposerTransport: () => ({
    commandsAvailable: () => true,
    sendMessage: harness.sendMessage,
    undoSend: harness.undoSend,
  }),
}));
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({
    error: harness.toastError,
    success: harness.toastSuccess,
    info: vi.fn(),
    warning: vi.fn(),
  }),
}));

import ComposeView from './ComposeView.vue';

const DRAFT_BODY = 'The quarterly figures are attached.';

function render(
  initialTo: string[] = ['boss@example.com'],
  embedded = true,
  props: Partial<InstanceType<typeof ComposeView>['$props']> = {},
) {
  return mount(ComposeView, {
    props: {
      embedded,
      initialTo,
      initialBody: DRAFT_BODY,
      ...props,
    },
    global: {
      plugins: [createPinia()],
      stubs: {
        AttachmentDropZone: true,
        ComposerSignature: true,
        ComposerThreadContext: true,
        RecipientChips: true,
        RichTextEditor: !harness.realEditor,
        SegmentedControl: true,
        SendLaterMenu: true,
        TemplatePicker: true,
      },
    },
  });
}

function sendButton(wrapper: ReturnType<typeof render>) {
  const button = wrapper.findAll('button').find(candidate => candidate.text() === 'Send');
  if (!button) throw new Error('Send button is missing');
  return button;
}

async function send(wrapper: ReturnType<typeof render>) {
  await sendButton(wrapper).trigger('click');
  await flushPromises();
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.templatesLoad.mockResolvedValue(undefined);
  harness.templatesLoadFailed = false;
  harness.fetchDraft.mockResolvedValue(null);
  harness.isPopout.mockReturnValue(false);
  harness.getByThread.mockReturnValue([]);
  harness.loadAttachmentsForMessages.mockResolvedValue(undefined);
  harness.composePlainText = true;
  harness.realEditor = false;
});

describe('composer fields and actions', () => {
  it('describes an empty subject and keeps Send primary while blocked', async () => {
    const wrapper = render([], false);
    await flushPromises();

    expect(wrapper.get('.compose-subject').attributes('placeholder')).toBe('Add a subject');
    expect(sendButton(wrapper).classes()).toContain('app-button--primary');
    expect(sendButton(wrapper).attributes('disabled')).toBeDefined();
    expect(wrapper.get('[data-testid="compose-send-wrap"]').attributes('title')).toBeTruthy();
    wrapper.unmount();
  });

  it('keeps the sender and recipients before the subject and body in DOM order', async () => {
    const wrapper = render(['boss@example.com'], true, {
      initialCc: ['copy@example.com'], initialBcc: ['private@example.com'],
    });
    await flushPromises();

    const fields = wrapper.findAll('select.sender-select, recipient-chips-stub, .compose-subject, .plain-editor');
    expect(fields.map((field) => field.element.tagName)).toEqual([
      'SELECT', 'RECIPIENT-CHIPS-STUB', 'RECIPIENT-CHIPS-STUB', 'RECIPIENT-CHIPS-STUB', 'INPUT', 'TEXTAREA',
    ]);
    expect((wrapper.get('select.sender-select').element as HTMLSelectElement).value).toBe('account-1|me@example.com');
    expect(wrapper.get('.footer').find('select').exists()).toBe(false);
  });

  it('names all utility actions and advertises the platform send shortcut', async () => {
    const wrapper = render();
    await flushPromises();

    for (const title of ['Attach file', 'Use template', 'Send later…', 'More options']) {
      expect(wrapper.get(`[title="${title}"]`).attributes('aria-label')).toBe(title);
    }
    expect(sendButton(wrapper).attributes('title')).toMatch(/^Send · /);
    expect(sendButton(wrapper).attributes('aria-keyshortcuts')).toMatch(/^(Control|Meta)\+Enter$/);
  });

  it('keeps the page header to the title and puts Save beside Send', async () => {
    const wrapper = render(['boss@example.com'], false);
    await flushPromises();

    expect(wrapper.get('.compose-header').findAll('button')).toHaveLength(0);
    const footerLabels = wrapper.get('.footer').findAll('button').map((button) => button.text());
    expect(footerLabels.slice(-3)).toEqual(['Save', 'Send', '']);
    expect(wrapper.get('[title="Attach file"]').text()).toBe('Attach');
    expect(wrapper.get('[title="Send later…"]').attributes('aria-haspopup')).toBe('menu');
    for (const button of wrapper.get('.footer-actions').findAll('button')) {
      expect(button.find('svg').exists()).toBe(true);
    }
  });
});

it.each(['reply', 'forward'] as const)('opens scheduling from the split control in an embedded %s', async (mode) => {
  const wrapper = render(['boss@example.com'], true, { mode });
  await flushPromises();
  const secondary = wrapper.get('.split-button__secondary');
  expect(secondary.attributes('aria-expanded')).toBe('false');
  await secondary.trigger('click');
  expect(secondary.attributes('aria-expanded')).toBe('true');
  const picker = wrapper.getComponent({ name: 'SendLaterMenu' });
  picker.vm.$emit('select', '2026-12-01T09:00:00Z');
  await flushPromises();
  expect(secondary.attributes('aria-expanded')).toBe('false');
  expect(wrapper.get('.split-button__primary').text()).toBe('Schedule');
  expect(wrapper.find('.split-button__primary circle').exists()).toBe(true);
  await wrapper.get('.schedule-info button').trigger('click');
  expect(sendButton(wrapper).find('svg path').exists()).toBe(true);
  wrapper.unmount();
});

describe.each([true, false])('outgoing conversation history, plain text: %s', (plainText) => {
  it.each(['reply', 'reply-all', 'forward'] as const)('includes earlier messages when sending a %s', async (mode) => {
    harness.composePlainText = plainText;
    harness.realEditor = !plainText;
    const messages: Message[] = [
      {
        id: 'original', threadId: 'thread-1',
        from: { name: 'Client', email: 'client@example.com' },
        to: [{ name: 'Me', email: 'me@example.com' }],
        subject: 'Project update', bodyText: 'Preview only', bodyLoaded: false,
        timestamp: '2026-10-13T10:00:54Z', isRead: true, attachments: [],
      },
      {
        id: 'sent', threadId: 'thread-1',
        from: { name: 'Me', email: 'me@example.com' },
        to: [{ name: 'Client', email: 'client@example.com' }],
        subject: 'Re: Project update', bodyText: 'Tuesday works for me.',
        timestamp: '2026-10-13T11:00:00Z', isRead: true, attachments: [],
      },
    ];
    harness.getByThread.mockReturnValue(messages);
    harness.loadAttachmentsForMessages.mockImplementation(async () => {
      harness.getByThread.mockReturnValue([
        { ...messages[0]!, bodyText: 'Can we meet on Tuesday?', bodyLoaded: true },
        messages[1]!,
      ]);
    });
    harness.sendMessage.mockResolvedValue({ status: 'sent', thread_id: 'thread-1', message_id: 'new' });
    const wrapper = render(undefined, true, { mode, threadId: 'thread-1', initialBody: 'Text before loading\n' });
    await flushPromises();

    expect(harness.loadAttachmentsForMessages).toHaveBeenCalledWith('thread-1', ['original']);
    if (plainText) {
      const editor = wrapper.get<HTMLTextAreaElement>('.plain-editor');
      await editor.setValue(`Thanks!\n${editor.element.value}`);
    } else {
      const editor = wrapper.getComponent(EditorContent).props('editor') as Editor;
      expect(editor.state.selection.from).toBe(1);
      editor.commands.insertContent('Thanks!');
      await flushPromises();
    }
    await send(wrapper);

    expect(harness.sendMessage).toHaveBeenCalledWith(expect.objectContaining({
      body: expect.stringContaining('Can we meet on Tuesday?'),
    }));
    const payload = harness.sendMessage.mock.calls[0]![0];
    expect(payload.body).toContain('Tuesday works for me.');
    expect(payload.body).not.toMatch(/\d{1,2}:\d{2}:\d{2}/);
    expect(payload.body).toContain('Text before loading');
    expect(payload.body).toContain('client@example.com');
    expect(payload.body).toContain('me@example.com');
    expect(payload.body.startsWith('Thanks!')).toBe(true);
    if (plainText) expect(payload.bodyHtml).toBeNull();
    else {
      expect(payload.bodyHtml).toContain('<blockquote>');
      expect(payload.bodyHtml).toContain('Can we meet on Tuesday?');
      expect(payload.bodyHtml).toContain('Tuesday works for me.');
    }

    const draftInput = harness.draftSetup.mock.calls[0]![0].buildInput()!;
    expect(draftInput.body).toBe(payload.body);
    expect(draftInput.bodyHtml).toBe(payload.bodyHtml);
    wrapper.unmount();

    harness.fetchDraft.mockResolvedValue({
      id: 'saved-reply', account_id: 'account-1', thread_id: 'thread-1',
      to: draftInput.to, cc: draftInput.cc, bcc: draftInput.bcc,
      subject: draftInput.subject, body: draftInput.body, body_html: draftInput.bodyHtml,
      send_options: draftInput.sendOptions,
    });
    const restored = render(undefined, true, {
      mode, threadId: 'thread-1', initialDraftId: 'saved-reply', initialBody: '',
    });
    await flushPromises();
    const restoredDraftInput = harness.draftSetup.mock.calls[1]![0].buildInput()!;
    expect(restoredDraftInput.body).toBe(payload.body);
    expect(restoredDraftInput.bodyHtml).toBe(payload.bodyHtml);
    await restored.get('button[title="Remove quoted history from this message"]').trigger('click');
    await flushPromises();
    expect(restored.find('.history-control').exists()).toBe(false);
    await send(restored);
    const restoredPayload = harness.sendMessage.mock.calls[1]![0];
    expect(restoredPayload.body).toContain('Thanks!');
    expect(restoredPayload.body).toContain('Text before loading');
    expect(restoredPayload.body).not.toContain('Can we meet on Tuesday?');
    expect(restoredPayload.body).not.toContain('Tuesday works for me.');
    expect(restoredPayload.bodyHtml ?? '').not.toContain('Can we meet on Tuesday?');
    const afterRemoval = harness.draftSetup.mock.calls[1]![0].buildInput()!;
    expect(afterRemoval.body).toBe(restoredPayload.body);
    expect(afterRemoval.bodyHtml).toBe(restoredPayload.bodyHtml);
    restored.unmount();
  });
});

it('keeps edits to the quoted history when switching between plain text and HTML', async () => {
  harness.composePlainText = false;
  harness.realEditor = true;
  harness.getByThread.mockReturnValue([{
    id: 'original', threadId: 'thread-1',
    from: { name: 'Client', email: 'client@example.com' }, to: [],
    subject: 'Project update', bodyText: 'Original conversation history',
    timestamp: '2026-10-13T10:00:00Z', isRead: true, attachments: [],
  }]);
  harness.sendMessage.mockResolvedValue({ status: 'sent', thread_id: 'thread-1', message_id: 'new' });
  const wrapper = render(undefined, true, { mode: 'reply', threadId: 'thread-1', initialBody: '' });
  await flushPromises();
  await wrapper.get('input[type="checkbox"]').setValue(true);
  await wrapper.get('.plain-editor').setValue('Reply with the history removed');
  await wrapper.get('[data-composer-history] .plain-editor').setValue('');
  await wrapper.get('input[type="checkbox"]').setValue(false);
  expect(wrapper.get('.ProseMirror').text()).toBe('Reply with the history removed');
  await send(wrapper);
  expect(harness.sendMessage.mock.calls[0]![0].body).toBe('Reply with the history removed');
  wrapper.unmount();
});

/**
 * One rejected side load used to leave `initialized` false for good: no dirty
 * tracking, no autosave, no close guard, and ten minutes of typing gone.
 */
describe('composer setup', () => {
  it.each(['imapUidValidityChanged', 'imapUidValidityMissing'].flatMap(key =>
    ['Drafts', 'offline'].map(folder => [key, folder]),
  ))('shows folder recovery when an imported draft fails with %s in %s', async (key, folder) => {
    harness.fetchDraft.mockRejectedValue({ key: `backend.errors.${key}`, params: { folder } });
    const wrapper = render([], true, { initialDraftId: 'draft-1', initialBody: '' });
    try {
      await flushPromises();
      const error = harness.toastError.mock.calls[0]![0] as string;
      expect(error).toContain(folder);
      expect(error).toMatch(/sync.*folder/i);
      expect(error).not.toContain('[object Object]');
      expect(sendButton(wrapper).attributes('disabled')).toBeDefined();
      expect(harness.draftSetup.mock.calls[0]![0].buildInput()).toBeNull();
    } finally {
      wrapper.unmount();
    }
  });

  it('refuses an unavailable list payload returned as a draft load', async () => {
    harness.fetchDraft.mockResolvedValue({ id: 'draft-1', account_id: 'account-1',
      to: [], cc: [], bcc: [], subject: '', body: '', updated_at: '',
      content_error: JSON.stringify({ key: 'backend.errors.imapUidValidityMissing', params: { folder: 'Work/Drafts' } }),
    });
    const wrapper = render([], true, { initialDraftId: 'draft-1', initialBody: '' });
    try {
      await flushPromises();
      expect(harness.toastError.mock.calls[0]![0]).toContain('Work/Drafts');
      expect(sendButton(wrapper).attributes('disabled')).toBeDefined();
      expect(harness.draftSetup.mock.calls[0]![0].buildInput()).toBeNull();
    } finally {
      wrapper.unmount();
    }
  });

  it('offers setup retry when the template store records a handled load failure', async () => {
    harness.templatesLoad.mockImplementationOnce(async () => { harness.templatesLoadFailed = true; });
    const wrapper = render();
    await flushPromises();
    expect(wrapper.get('.compose-notices').text()).toContain('templates');
    harness.templatesLoadFailed = false;
    await wrapper.get('.compose-notices button').trigger('click');
    await flushPromises();
    expect(harness.templatesLoad).toHaveBeenCalledTimes(2);
    expect(wrapper.find('.compose-notices').exists()).toBe(false);
    wrapper.unmount();
  });

  it('keeps autosave and the close guard when a side load fails, and offers a retry', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    harness.templatesLoad.mockRejectedValueOnce(new Error('list_templates timed out'));
    const wrapper = render();
    await flushPromises();

    expect(harness.startAutosave).toHaveBeenCalledTimes(1);
    expect(harness.captureBaseline).toHaveBeenCalledTimes(1);
    const banner = wrapper.get('.compose-notices');
    expect(banner.text()).toContain('Some options could not be loaded');
    expect(banner.text()).toContain('templates');

    await banner.get('button').trigger('click');
    await flushPromises();

    expect(harness.templatesLoad).toHaveBeenCalledTimes(2);
    expect(wrapper.find('.compose-notices').exists()).toBe(false);
  });
});

describe('composer send failure', () => {
  it('announces uncertain delivery without claiming a retry was queued', async () => {
    harness.sendMessage.mockResolvedValue({ status: 'delivery_unknown', thread_id: 'th-1', message_id: 'queued-1' });
    const wrapper = render();
    await flushPromises();
    await send(wrapper);
    expect(harness.toastError).toHaveBeenCalled();
    expect(harness.resume).not.toHaveBeenCalled();
    expect(wrapper.emitted('sent')).toHaveLength(1);
  });

  it('says the message did not go out, in words and in tone', async () => {
    harness.sendMessage.mockRejectedValue(new Error('invalid_grant: token expired'));
    const wrapper = render();
    await flushPromises();

    await send(wrapper);

    const alert = wrapper.get('[role="alert"]');
    expect(alert.classes()).toContain('status--error');
    expect(alert.text()).toContain(
      "This account's sign-in has expired. Please reconnect the account in the settings.",
    );
    expect(alert.text()).not.toContain('invalid_grant');
  });

  it('keeps the raw cause reachable but collapsed', async () => {
    harness.sendMessage.mockRejectedValue(new Error('invalid_grant: token expired'));
    const wrapper = render();
    await flushPromises();

    await send(wrapper);

    expect(wrapper.get('details pre').text()).toBe('invalid_grant: token expired');
  });

  it('raises exactly one toast, which outlives the composer', async () => {
    harness.sendMessage.mockRejectedValue(new Error('invalid_grant: token expired'));
    const wrapper = render();
    await flushPromises();

    await send(wrapper);

    expect(harness.toastError).toHaveBeenCalledTimes(1);
    expect(harness.toastError.mock.calls[0]![0]).toContain("This account's sign-in has expired");
  });

  it('leaves the composer open with the draft and its autosave intact', async () => {
    harness.sendMessage.mockRejectedValue(new Error('invalid_grant: token expired'));
    const wrapper = render();
    await flushPromises();

    await send(wrapper);

    expect(wrapper.find('section.compose').exists()).toBe(true);
    expect(wrapper.get('textarea').element.value).toBe(DRAFT_BODY);
    expect(harness.resume).toHaveBeenCalledTimes(1);
    expect(wrapper.emitted('sent')).toBeUndefined();
  });

  it('stays quiet when the message goes out', async () => {
    harness.sendMessage.mockResolvedValue({
      status: 'sent',
      thread_id: 'thread-1',
      message_id: 'message-1',
    } as BackendSendMessageResult);
    const wrapper = render();
    await flushPromises();

    await send(wrapper);

    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(harness.toastError).not.toHaveBeenCalled();
    expect(wrapper.emitted('sent')).toHaveLength(1);
  });

  it('confirms the send after the docked composer closes', async () => {
    harness.sendMessage.mockResolvedValue({
      status: 'sent',
      thread_id: 'thread-1',
      message_id: 'message-1',
    } as BackendSendMessageResult);
    harness.toastSuccess.mockClear();
    const wrapper = render();
    await flushPromises();

    await send(wrapper);

    // The status line closes with the window; the toast outlives it.
    expect(harness.toastSuccess).toHaveBeenCalledWith('Mail sent.');
  });
});

describe('composer send guard', () => {
  it('refuses to send while a chip holds no address', async () => {
    const wrapper = render(['"Buck']);
    await flushPromises();

    expect(sendButton(wrapper).attributes('disabled')).toBeDefined();
    await send(wrapper);
    expect(harness.sendMessage).not.toHaveBeenCalled();
  });

  it('sends once every chip parses as a mailbox', async () => {
    harness.sendMessage.mockResolvedValue({
      status: 'sent',
      thread_id: 'thread-1',
      message_id: 'message-1',
    } as BackendSendMessageResult);
    const wrapper = render(['"Morgan, Alex" <a.morgan@example.com>']);
    await flushPromises();

    expect(sendButton(wrapper).attributes('disabled')).toBeUndefined();
    await send(wrapper);
    expect(harness.sendMessage).toHaveBeenCalledTimes(1);
  });

  it('says why Send is disabled instead of only greying it out', async () => {
    // Not a blank new message: that one moves the focus into the (stubbed) To field.
    const wrapper = render([], true, { mode: 'reply' });
    await flushPromises();

    expect(sendButton(wrapper).attributes('disabled')).toBeDefined();
    const hint = 'Add a recipient under To, Cc or Bcc.';
    expect(wrapper.get('[data-testid="compose-send-wrap"]').attributes('title')).toBe(hint);
    const describedBy = sendButton(wrapper).attributes('aria-describedby');
    expect(wrapper.get(`#${describedBy}`).text()).toBe(hint);
    const secondary = wrapper.get('.split-button__secondary');
    expect(secondary.attributes('disabled')).toBeDefined();
    expect(secondary.attributes('aria-describedby')).toBe(describedBy);
  });

  it('names the invalid address in the hint', async () => {
    const wrapper = render(['"Buck']);
    await flushPromises();
    expect(wrapper.get('[data-testid="compose-send-wrap"]').attributes('title'))
      .toBe('Not a valid email address: "Buck');
  });

  it.each(['cc', 'bcc'] as const)('sends a message addressed through %s alone', async (field) => {
    harness.sendMessage.mockResolvedValue({
      status: 'sent', thread_id: 'thread-1', message_id: 'message-1',
    } as BackendSendMessageResult);
    const address = 'copy@example.com';
    const wrapper = render([], true, field === 'cc'
      ? { mode: 'reply', initialCc: [address] }
      : { mode: 'reply', initialBcc: [address] });
    await flushPromises();

    expect(sendButton(wrapper).attributes('disabled')).toBeUndefined();
    await send(wrapper);
    expect(harness.sendMessage).toHaveBeenCalledWith(expect.objectContaining({ to: [], [field]: [address] }));
  });

  it('sends a message that consists of attachments only', async () => {
    harness.sendMessage.mockResolvedValue({
      status: 'sent', thread_id: 'thread-1', message_id: 'message-1',
    } as BackendSendMessageResult);
    harness.fetchDraft.mockResolvedValue({
      id: 'draft-1', account_id: 'account-1', to: ['boss@example.com'], cc: [], bcc: [],
      subject: 'Scan', body: '', body_html: null,
      attachments: [{ filename: 'scan.pdf', mime_type: 'application/pdf', content_base64: 'JVBERi0=', inline: false }],
    });
    const wrapper = render([], true, { initialDraftId: 'draft-1', initialBody: '' });
    await flushPromises();

    expect(sendButton(wrapper).attributes('disabled')).toBeUndefined();
    await send(wrapper);
    expect(harness.sendMessage).toHaveBeenCalledWith(expect.objectContaining({
      body: '',
      attachments: [expect.objectContaining({ filename: 'scan.pdf' })],
    }));
  });

  it.each(['meta', 'ctrl'] as const)('sends with %s+Enter from the subject line', async (modifier) => {
    harness.sendMessage.mockResolvedValue({
      status: 'sent', thread_id: 'thread-1', message_id: 'message-1',
    } as BackendSendMessageResult);
    const wrapper = render();
    await flushPromises();

    await wrapper.get('.compose-subject').trigger(`keydown.${modifier}.enter`);
    await flushPromises();
    expect(harness.sendMessage).toHaveBeenCalledTimes(1);
  });

  it('sends when the recipient field submits', async () => {
    harness.sendMessage.mockResolvedValue({
      status: 'sent', thread_id: 'thread-1', message_id: 'message-1',
    } as BackendSendMessageResult);
    const wrapper = render();
    await flushPromises();

    wrapper.getComponent({ name: 'RecipientChips' }).vm.$emit('submit');
    await flushPromises();
    expect(harness.sendMessage).toHaveBeenCalledTimes(1);
  });

  it('explains a blocked shortcut send in the status line', async () => {
    // Not a blank new message: that one moves the focus into the (stubbed) To field.
    const wrapper = render([], true, { mode: 'reply' });
    await flushPromises();

    await wrapper.get('.compose-subject').trigger('keydown.meta.enter');
    await flushPromises();
    expect(harness.sendMessage).not.toHaveBeenCalled();
    expect(wrapper.get('.status--warning').text()).toBe('Add a recipient under To, Cc or Bcc.');
  });
});

/**
 * `n` followed by an address used to put the address in the message: the
 * editor took the focus whether or not the composer already knew who it was
 * going to.
 */
describe('where a fresh composer starts typing', () => {
  const focusRecipients = vi.fn();

  const RecipientChipsStub = defineComponent({
    name: 'RecipientChips',
    props: { modelValue: { type: Array, default: () => [] }, placeholder: String },
    setup(_props, { expose }) {
      expose({ focus: focusRecipients });
      return () => h('div', { class: 'recipient-chips-stub' });
    },
  });

  function renderWith(initialTo: string[]) {
    return mount(ComposeView, {
      props: { embedded: true, initialTo, initialBody: DRAFT_BODY },
      global: {
        stubs: {
          AttachmentDropZone: true,
          ComposerSignature: true,
          ComposerThreadContext: true,
          RecipientChips: RecipientChipsStub,
          RichTextEditor: true,
          SegmentedControl: true,
          SendLaterMenu: true,
          TemplatePicker: true,
        },
      },
    });
  }

  beforeEach(() => focusRecipients.mockClear());

  it('starts in the To field when there is no recipient yet', async () => {
    renderWith([]);
    await flushPromises();

    expect(focusRecipients).toHaveBeenCalledTimes(1);
  });

  it('leaves the focus to the editor when the recipient is already known', async () => {
    renderWith(['boss@example.com']);
    await flushPromises();

    expect(focusRecipients).not.toHaveBeenCalled();
  });
});

/**
 * A popout used to route itself into a mailbox it had no stores for, and the
 * Undo toast appeared only in the main window.
 */
describe('sending from a popout', () => {
  it('closes the window once the message is sent', async () => {
    harness.isPopout.mockReturnValue(true);
    harness.sendMessage.mockResolvedValue({
      status: 'sent',
      thread_id: 'thread-1',
      message_id: 'message-1',
    } as BackendSendMessageResult);
    const wrapper = render(['boss@example.com'], false);
    await flushPromises();

    await send(wrapper);

    expect(harness.closePopout).toHaveBeenCalledTimes(1);
    expect(harness.push).not.toHaveBeenCalled();
  });

  it('keeps Undo in the popout until the send window has passed', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      harness.isPopout.mockReturnValue(true);
      harness.sendMessage.mockResolvedValue({
        status: 'queued',
        thread_id: 'th-pending-q1',
        message_id: 'queued-q1',
        queued_id: 'q1',
        undo_until: new Date(Date.now() + 5_000).toISOString(),
      } as BackendSendMessageResult);
      const wrapper = render(['boss@example.com'], false);
      await flushPromises();

      await send(wrapper);

      const banner = wrapper.get('.compose-after-send');
      expect(banner.text()).toContain('Sending in 5 seconds');
      expect(banner.text()).toContain('Undo');
      expect(harness.closePopout).not.toHaveBeenCalled();
      expect(harness.push).not.toHaveBeenCalled();

      await vi.advanceTimersByTimeAsync(5_500);
      expect(harness.closePopout).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('still routes the main window\'s full-page composer as before', async () => {
    harness.sendMessage.mockResolvedValue({
      status: 'sent',
      thread_id: 'thread-1',
      message_id: 'message-1',
    } as BackendSendMessageResult);
    const wrapper = render(['boss@example.com'], false);
    await flushPromises();

    await send(wrapper);

    expect(harness.closePopout).not.toHaveBeenCalled();
    expect(harness.push).toHaveBeenCalledWith({ name: 'thread', params: { id: 'thread-1' } });
  });
});

it('retains HTML composition for an untouched new-message prefill', async () => {
  harness.composePlainText = false;
  harness.realEditor = true;
  harness.sendMessage.mockResolvedValue({ status: 'sent', thread_id: 'new', message_id: 'new' });
  const wrapper = render(undefined, true, { initialBody: 'Prefilled new message', initialBodyHtml: null });
  await flushPromises();
  const draft = harness.draftSetup.mock.calls[0]![0].buildInput()!;
  expect(draft.bodyHtml).toBe('<p>Prefilled new message</p>');
  await send(wrapper);
  expect(harness.sendMessage.mock.calls[0]![0].bodyHtml).toBe(draft.bodyHtml);
  wrapper.unmount();
});

/**
 * Reply, `r`, the draft card and the drafts list all open a saved draft as a
 * new message; only the loaded draft knows it answers something. The thread
 * draft in the mock used to carry neither HTML nor a signature nor the message
 * it answers, so resuming it showed a bare plain-text form headed "New message".
 */
it('resumes the mock thread draft as the reply it was saved as', async () => {
  const { mockDrafts } = await import('@/data/mock');
  const saved = mockDrafts.find((draft) => draft.id === 'dr-offer')!;
  harness.composePlainText = true;
  harness.realEditor = true;
  harness.fetchDraft.mockResolvedValue({ ...saved });
  const wrapper = render([], true, { mode: 'new', initialDraftId: saved.id, initialBody: '' });
  await flushPromises();

  expect(wrapper.emitted('update:mode')).toEqual([['reply']]);
  expect(wrapper.emitted('update:title')?.at(-1)).toEqual(['Re: Staging login 500 after deployment']);
  expect(wrapper.find('textarea.plain-editor').exists()).toBe(false);
  const [writing, history] = wrapper.findAll('.ProseMirror');
  expect(writing!.text()).toBe('Der OAuth-Fix ist auf Staging. Kannst du den Login noch einmal gegenprüfen?');
  expect(wrapper.find('[data-composer-history]').exists()).toBe(true);
  expect(history!.text()).toContain('Users cannot sign in on staging since this morning.');

  const input = harness.draftSetup.mock.calls[0]![0].buildInput()!;
  expect(input.signatureId).toBe('signature-acc-work-0');
  expect(input.sendOptions?.reply_to_message_id).toBe('m-1');
  wrapper.unmount();
});


it.each(['restored', 'tick', 'submit'])('blocks an expired schedule on %s', async (mode) => {
  const restored = mode === 'restored';
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
  vi.setSystemTime(new Date('2026-10-29T12:00:00Z'));
  if (restored) harness.fetchDraft.mockResolvedValue({
    id: 'expired', account_id: 'account-1', to: ['boss@example.com'], cc: [], bcc: [],
    subject: 'Expired schedule', body: DRAFT_BODY, scheduled_for: '2026-10-29T11:59:00Z',
  });
  const wrapper = render(undefined, true, restored ? { initialDraftId: 'expired' } : {});
  await flushPromises();
  if (!restored) {
    await wrapper.get('.split-button__secondary').trigger('click');
    await flushPromises();
    wrapper.getComponent({ name: 'SendLaterMenu' }).vm.$emit('select', '2026-10-29T12:01:00Z');
    await flushPromises();
    expect(wrapper.get('.split-button__primary').attributes('disabled')).toBeUndefined();
    if (mode === 'tick') await vi.advanceTimersByTimeAsync(60_000);
    else {
      // Submission must revalidate even between clock updates or after sleep.
      vi.setSystemTime(new Date('2026-10-29T12:01:00Z'));
      wrapper.getComponent({ name: 'RecipientChips' }).vm.$emit('submit');
      await flushPromises();
    }
  }
  expect(wrapper.get('.split-button__primary').attributes('disabled')).toBeDefined();
  expect(wrapper.get('[data-testid="compose-send-wrap"]').attributes('title')).toContain('future');
  wrapper.getComponent({ name: 'RecipientChips' }).vm.$emit('submit');
  await flushPromises();
  expect(harness.sendMessage).not.toHaveBeenCalled();
  const later = wrapper.get('.split-button__secondary');
  expect(later.attributes('disabled')).toBeUndefined();
  await later.trigger('click');
  await flushPromises();
  wrapper.getComponent({ name: 'SendLaterMenu' }).vm.$emit('select', '2026-10-29T13:00:00Z');
  await flushPromises();
  expect(wrapper.get('.split-button__primary').attributes('disabled')).toBeUndefined();
  await wrapper.get('.schedule-info button').trigger('click');
  expect(sendButton(wrapper).attributes('disabled')).toBeUndefined();
  wrapper.unmount();
  vi.useRealTimers();
});
