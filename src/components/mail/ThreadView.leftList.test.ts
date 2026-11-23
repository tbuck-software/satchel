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
    workflowState: 'done',
    likelySpam: false,
  };
  return {
    thread,
    messages: [] as Array<Record<string, unknown>>,
    messagesLoading: false,
    messagesError: null as string | null,
    archive: vi.fn(),
    blockSender: vi.fn().mockResolvedValue(undefined),
    markNotSpam: vi.fn().mockResolvedValue(undefined),
    moveToInbox: vi.fn().mockResolvedValue(undefined),
    store: { activeThreadLeftList: true, activeThreadId: 'thread-1' },
    showInReader: vi.fn(),
    closeReader: vi.fn(),
    toastError: vi.fn(),
    loadMessages: vi.fn().mockResolvedValue(undefined),
  };
});

vi.mock('@/stores/connection', () => ({
  useConnectionStore: () => ({ backendErrorFor: () => harness.messagesError }),
}));
vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => ({
    accounts: [{ id: 'account-1', email: 'me@example.com', displayName: 'Me' }],
    capabilitiesFor: () => ({ can_reply: false }),
  }),
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
  useSettingsStore: () => ({ readerZoom: 100, setReaderZoom: vi.fn(), keybindings: {}, singleKeyShortcuts: true }),
}));
vi.mock('@/stores/mailboxes', () => ({
  useMailboxesStore: () => ({ mailboxes: [], load: vi.fn() }),
}));
vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => ({
    showInReader: harness.showInReader,
    closeReader: harness.closeReader,
    get activeThreadLeftList() { return harness.store.activeThreadLeftList; },
    get activeThreadId() { return harness.store.activeThreadId; },
    threads: [harness.thread],
    find: (id: string) => (id === harness.thread.id ? harness.thread : undefined),
    scopedVisibleThreads: [],
    focusNeighborThread: vi.fn(),
    setThreadsRead: vi.fn(),
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
    canTrash: () => true,
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
    blockSender: harness.blockSender,
    markNotSpam: harness.markNotSpam,
  }),
}));
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), info: vi.fn(), error: harness.toastError }),
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
    props: { threadId: harness.thread.id, showBack: false },
    global: {
      stubs: {
        MessageBubble: true,
        PluginSlotHost: true,
        SnoozePicker: true,
        TopicChip: true,
        ThreadPaneSkeleton: true,
        ThreadToolbar: true,
      },
    },
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.thread.workflowState = 'done';
  harness.store.activeThreadLeftList = true;
  harness.store.activeThreadId = 'thread-1';
  harness.messages = [{
    id: 'm1',
    threadId: 'thread-1',
    from: { name: 'Hotel', email: 'front-desk@example.test' },
    to: [],
    subject: 'Design review',
    bodyText: '',
    bodyLoaded: true,
    isDraft: false,
    isRead: true,
    timestamp: '2026-08-13T18:00:00Z',
    attachments: [],
  }];
});

describe('a thread that left the list while open', () => {
  it('tells the reader it is filed and when it comes back, with the way back', async () => {
    const wrapper = render();
    await flushPromises();

    const banner = wrapper.find('.thread-left-list-banner');
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain('Done');
    expect(banner.text()).toContain('It comes back to the inbox when someone replies.');
    const button = banner.findAll('button').find((candidate) => candidate.text() === 'Back to the inbox');
    expect(button).toBeDefined();

    await button!.trigger('click');
    expect(harness.moveToInbox).toHaveBeenCalledWith(['thread-1']);
  });

  it('offers no way back for a thread that came back on its own', async () => {
    harness.thread.workflowState = 'inbox';
    const wrapper = render();
    await flushPromises();

    const banner = wrapper.find('.thread-left-list-banner');
    expect(banner.text()).toContain('Inbox');
    expect(banner.text()).toContain('It leaves this list once you move on.');
    expect(banner.find('button').exists()).toBe(false);
  });

  it('stays away while the thread still belongs to the list', async () => {
    harness.store.activeThreadLeftList = false;
    const wrapper = render();
    await flushPromises();
    expect(wrapper.find('.thread-left-list-banner').exists()).toBe(false);
  });

  it('tells the store which thread it shows, and lets go on unmount', async () => {
    const wrapper = render();
    await flushPromises();
    expect(harness.showInReader).toHaveBeenCalledWith('thread-1');

    wrapper.unmount();
    expect(harness.closeReader).toHaveBeenCalledWith('thread-1');
  });
});
