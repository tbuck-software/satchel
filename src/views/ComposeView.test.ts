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
  useTemplatesStore: () => ({ load: harness.templatesLoad, items: [] }),
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
        SendLaterPicker: true,
        TemplatePicker: true,
      },
    },
  });
}

async function send(wrapper: ReturnType<typeof render>) {
  await wrapper.get('.primary-btn').trigger('click');
  await flushPromises();
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.templatesLoad.mockResolvedValue(undefined);
  harness.fetchDraft.mockResolvedValue(null);
  harness.isPopout.mockReturnValue(false);
  harness.getByThread.mockReturnValue([]);
  harness.loadAttachmentsForMessages.mockResolvedValue(undefined);
  harness.composePlainText = true;
  harness.realEditor = false;
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
        timestamp: '2026-10-13T10:00:00Z', isRead: true, attachments: [],
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
});

describe('composer send guard', () => {
  it('refuses to send while a chip holds no address', async () => {
    const wrapper = render(['"Buck']);
    await flushPromises();

    expect(wrapper.get('.primary-btn').attributes('disabled')).toBeDefined();
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

    expect(wrapper.get('.primary-btn').attributes('disabled')).toBeUndefined();
    await send(wrapper);
    expect(harness.sendMessage).toHaveBeenCalledTimes(1);
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
          SendLaterPicker: true,
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
