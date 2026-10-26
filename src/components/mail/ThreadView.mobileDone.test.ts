// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => {
  const thread = {
    id: 'thread-1',
    accountId: 'account-1',
    subject: 'Design review',
    mailboxNames: ['Inbox'],
    unreadCount: 1,
    isStarred: false,
    isMuted: false,
    tags: [],
    workflowState: 'inbox',
  };
  return {
    thread,
    messages: [] as Array<Record<string, unknown>>,
    messagesLoading: false,
    messagesError: null as string | null,
    archive: vi.fn(),
    moveToInbox: vi.fn(),
    loadMessages: vi.fn().mockResolvedValue(undefined),
  };
});

vi.mock('@/stores/connection', () => ({
  useConnectionStore: () => ({ backendErrorFor: () => harness.messagesError }),
}));
vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => ({ accounts: [], capabilitiesFor: () => ({ can_reply: false }) }),
}));
vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({
    getByThread: () => harness.messages,
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
    threads: [harness.thread],
    find(this: { threads: Array<{ id: string }> }, id: string) {
      return this.threads.find((thread) => thread.id === id);
    },
    scopedVisibleThreads: [],
    focusNeighborThread: vi.fn(),
    setThreadsRead: vi.fn(),
  }),
}));
vi.mock('@/stores/topics', () => ({
  useTopicsStore: () => ({
    topicIdsFor: () => [],
    hasMembershipsFor: () => false,
    byId: vi.fn(),
    loadMembershipsFor: vi.fn(),
    openPicker: vi.fn(),
  }),
}));
vi.mock('@/stores/ui', () => ({
  useUiStore: () => ({ isMobile: true, openOmni: vi.fn() }),
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
    setRead: vi.fn().mockResolvedValue(true),
    setMuted: vi.fn().mockResolvedValue(true),
    markSpam: vi.fn().mockResolvedValue(true),
    archive: harness.archive,
    trash: vi.fn(),
    removeFromTopic: vi.fn(),
    snoozeAt: vi.fn(),
    togglePinned: vi.fn(),
    setAside: vi.fn(),
    moveToInbox: harness.moveToInbox,
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
vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }));

import ThreadView from './ThreadView.vue';

function render() {
  return mount(ThreadView, {
    props: { threadId: harness.thread.id, showBack: true },
    global: {
      stubs: {
        MessageBubble: true,
        PluginSlotHost: true,
        SnoozePicker: true,
        TopicChip: true,
        ThreadPaneSkeleton: { template: '<div data-test="thread-skeleton" />' },
      },
    },
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.thread.workflowState = 'inbox';
  harness.messages = [];
  harness.messagesLoading = false;
  harness.loadMessages.mockResolvedValue(undefined);
});

describe('mobile Done action', () => {
  it('leaves the routed detail immediately while the transition continues', async () => {
    let finish!: (completed: boolean) => void;
    harness.archive.mockReturnValue(new Promise<boolean>((resolve) => { finish = resolve; }));
    const wrapper = render();
    await flushPromises();

    const button = wrapper.get('.bar-done');
    await button.trigger('click');

    expect(harness.archive).toHaveBeenCalledWith(['thread-1']);
    expect(wrapper.emitted('back')).toHaveLength(1);
    expect(button.text()).toBe('Done');

    finish(true);
    await flushPromises();

    expect(wrapper.emitted('back')).toHaveLength(1);
  });

  it('does not wait for a failed background transition before leaving', async () => {
    harness.archive.mockResolvedValue(false);
    const wrapper = render();
    await flushPromises();

    const button = wrapper.get('.bar-done');
    await button.trigger('click');
    await flushPromises();

    expect(wrapper.emitted('back')).toHaveLength(1);
  });

  it('offers the way back to the inbox on an already completed conversation', async () => {
    harness.thread.workflowState = 'done';
    const wrapper = render();
    await flushPromises();

    expect(wrapper.get('.thread-state-badge').text()).toBe('Done');
    const button = wrapper.get('.bar-done');
    expect(button.text()).toBe('Move to inbox');
    await button.trigger('click');

    expect(harness.moveToInbox).toHaveBeenCalledWith(['thread-1']);
    expect(harness.archive).not.toHaveBeenCalled();
    expect(wrapper.emitted('back')).toBeUndefined();
  });

  it('keeps the initial conversation skeleton until the visible body is ready', () => {
    harness.messages = [{
      id: 'message-1',
      threadId: 'thread-1',
      from: { name: 'Sender', email: 'sender@example.com' },
      to: [{ name: 'Me', email: 'me@example.com' }],
      subject: 'Design review',
      bodyText: '',
      bodyLoaded: false,
      isDraft: false,
      isRead: true,
      timestamp: '2026-08-13T18:00:00Z',
      attachments: [],
    }];
    harness.messagesLoading = true;

    const wrapper = render();

    expect(wrapper.find('[data-test="thread-skeleton"]').exists()).toBe(true);
    expect(wrapper.find('message-bubble-stub').exists()).toBe(false);
  });
});
