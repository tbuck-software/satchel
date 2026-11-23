// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  messages: [] as Array<Record<string, unknown>>,
  messagesLoading: false,
  messagesError: null as string | null,
  loadMessages: vi.fn().mockResolvedValue(undefined),
  exportMessage: vi.fn().mockResolvedValue(false),
  messagesForExport: vi.fn(),
  copyText: vi.fn(),
}));

vi.mock('@/stores/connection', () => ({
  useConnectionStore: () => ({
    backendErrorFor: (source: string) => (source === 'messages' ? harness.messagesError : null),
  }),
}));
vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => ({ accounts: [], capabilitiesFor: () => ({ can_reply: false }) }),
}));
vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({
    getByThread: () => harness.messages,
    exportMessage: harness.exportMessage,
    messagesForExport: harness.messagesForExport,
    isLoading: () => harness.messagesLoading,
    loadByThread: harness.loadMessages,
    openThread: harness.loadMessages,
    isBodyLoading: () => false,
    hasThread: () => false,
    retainThread: () => () => {},
    needsDetails: () => false,
    loadAttachmentsForMessages: vi.fn(),
  }),
}));
vi.mock('@/stores/settings', () => ({
  useSettingsStore: () => ({ readerZoom: 100, setReaderZoom: vi.fn() }),
}));
vi.mock('@/stores/mailboxes', () => ({
  useMailboxesStore: () => ({ mailboxes: [], load: vi.fn() }),
}));
vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => ({
    showInReader: vi.fn(),
    closeReader: vi.fn(),
    threads: [{
      id: 'thread-1',
      accountId: 'account-1',
      subject: 'Design review',
      mailboxNames: ['Inbox'],
      unreadCount: 0,
      isStarred: false,
      isMuted: false,
      tags: [],
      workflowState: 'inbox',
    }],
    find(this: { threads: Array<{ id: string }> }, id: string) {
      return this.threads.find((thread) => thread.id === id);
    },
    scopedVisibleThreads: [],
    focusNeighborThread: vi.fn(),
    setThreadsRead: vi.fn(),
    load: vi.fn(),
  }),
}));
vi.mock('@/stores/topics', () => ({
  useTopicsStore: () => ({
    membershipsKnownFor: () => true, membershipsFailedFor: () => false, membershipsLoadingFor: () => false, topicIdsFor: () => [],
    hasMembershipsFor: () => false,
    byId: vi.fn(),
    loadMembershipsFor: vi.fn(),
    openPicker: vi.fn(),
  }),
}));
vi.mock('@/stores/ui', () => ({
  useUiStore: () => ({ isMobile: false, openOmni: vi.fn() }),
}));
vi.mock('@/stores/composer', () => ({
  useComposerStore: () => ({
    openReply: vi.fn(),
    openForward: vi.fn(),
    openResend: vi.fn(),
    openDraft: vi.fn(),
  }),
}));
vi.mock('@/composables/useDraftActions', () => ({
  useDraftActions: () => ({ discard: vi.fn(), isBusy: () => false }),
}));
vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({
    canTrash: () => true,
    setRead: vi.fn().mockResolvedValue(true),
    setMuted: vi.fn().mockResolvedValue(true),
    markSpam: vi.fn().mockResolvedValue(true),
    archive: vi.fn(),
    trash: vi.fn(),
    removeFromTopic: vi.fn(),
    snoozeAt: vi.fn(),
    togglePinned: vi.fn(),
    setAside: vi.fn(),
    moveToInbox: vi.fn(),
  }),
}));
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), info: vi.fn(), error: vi.fn() }),
}));
vi.mock('@/services/backend', () => ({
  backendBlockSender: vi.fn(),
  backendDownloadAttachment: vi.fn(),
  backendMoveThreadsToFolder: vi.fn(),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
}));
vi.mock('@/utils/perf', () => ({ perfStart: () => undefined }));
vi.mock('@/services/clipboard', () => ({ writeClipboardText: harness.copyText }));
vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }));

import ThreadView from './ThreadView.vue';
import { activeThreadReaderCommands } from '@/composables/useThreadReaderCommands';

function render() {
  return mount(ThreadView, {
    props: { threadId: 'thread-1' },
    global: {
      stubs: {
        MessageBubble: true,
        ThreadToolbar: true,
        PluginSlotHost: true,
        SnoozePicker: true,
        TopicChip: true,
        ThreadMessageList: { template: '<div data-test="message-list" />' },
        ThreadPaneSkeleton: { template: '<div data-test="thread-skeleton" />' },
      },
    },
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.messages = [];
  harness.messagesLoading = false;
  harness.messagesError = null;
  harness.loadMessages.mockResolvedValue(undefined);
  harness.messagesForExport.mockImplementation(async (_threadId, ids: string[]) =>
    harness.messages.filter(message => ids.includes(message.id as string)));
});

describe('reader load failure', () => {
  it('waits for complete bodies before copying Markdown from the toolbar', async () => {
    const preview = { id: 'old', from: { email: 'sender@example.test' }, to: [],
      bodyText: 'Need h…', bodyLoaded: false, isDraft: false, isRead: true,
      timestamp: '2026-08-13T18:00:00Z', attachments: [] };
    harness.messages = [preview];
    harness.messagesForExport.mockResolvedValue([{ ...preview, bodyLoaded: true,
      bodyHtml: '<p>Complete footer</p><a href="https://example.test/login">Sign in</a>' }]);
    const wrapper = render();
    await flushPromises();
    wrapper.getComponent({ name: 'ThreadToolbar' }).vm.$emit('copy-markdown', 'conversation');
    await flushPromises();
    expect(harness.messagesForExport).toHaveBeenCalledWith('thread-1', ['old']);
    expect(harness.copyText).toHaveBeenCalledWith(expect.stringContaining('Complete footer'));
    expect(harness.copyText).toHaveBeenCalledWith(expect.stringContaining('[Sign in](https://example.test/login)'));
    expect(harness.copyText).not.toHaveBeenCalledWith(expect.stringContaining('Need h…'));
    wrapper.unmount();
  });

  it('routes the existing EML menu action to the complete original export', async () => {
    harness.messages = [{ id: 'original-message', from: { email: 'sender@example.test' }, to: [],
      bodyText: 'Display text only', bodyHtml: '<p>Original is richer</p>', bodyLoaded: true,
      isDraft: false, isRead: true, timestamp: '2026-08-13T18:00:00Z', attachments: [] }];
    const wrapper = render();
    await flushPromises();
    await activeThreadReaderCommands()!.exportThread('eml');
    expect(harness.exportMessage).toHaveBeenCalledExactlyOnceWith('original-message', 'Design review');
    wrapper.unmount();
  });

  it('replaces the blank message area with the failure and a way back', async () => {
    harness.messagesError = 'backend.errors.messageReadFailed';
    const wrapper = render();
    await flushPromises();

    const alert = wrapper.get('[role="alert"]');
    expect(alert.text()).toContain('This conversation could not be loaded.');
    expect(alert.text()).toContain('The message could not be loaded. Please try again.');
    expect(alert.text()).not.toContain('backend.errors.');
    expect(wrapper.find('[data-test="message-list"]').exists()).toBe(false);
  });

  it('retries on the same path the conversation opened with', async () => {
    harness.messagesError = 'backend.errors.messageReadFailed';
    const wrapper = render();
    await flushPromises();
    harness.loadMessages.mockClear();

    await wrapper.get('[role="alert"] button').trigger('click');

    expect(harness.loadMessages).toHaveBeenCalledWith('thread-1');
  });

  it('shows the messages, not the failure, once a load succeeds', async () => {
    harness.messages = [{
      id: 'message-1',
      threadId: 'thread-1',
      from: { name: 'Sender', email: 'sender@example.com' },
      to: [{ name: 'Me', email: 'me@example.com' }],
      subject: 'Design review',
      bodyText: 'Body',
      bodyLoaded: true,
      isDraft: false,
      isRead: true,
      timestamp: '2026-08-13T18:00:00Z',
      attachments: [],
    }];
    const wrapper = render();
    await flushPromises();

    expect(wrapper.find('[data-test="message-list"]').exists()).toBe(true);
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  });
});
