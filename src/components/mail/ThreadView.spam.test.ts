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
    likelySpam: true,
  };
  return {
    thread,
    messages: [] as Array<Record<string, unknown>>,
    messagesLoading: false,
    messagesError: null as string | null,
    archive: vi.fn(),
    blockSender: vi.fn().mockResolvedValue(undefined),
    markNotSpam: vi.fn().mockResolvedValue(undefined),
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
    setRead: vi.fn().mockResolvedValue(true),
    setMuted: vi.fn().mockResolvedValue(true),
    markSpam: vi.fn().mockResolvedValue(true),
    archive: harness.archive,
    trash: vi.fn(),
    removeFromTopic: vi.fn(),
    snoozeAt: vi.fn(),
    togglePinned: vi.fn(),
    setAside: vi.fn(),
    moveToInbox: vi.fn(),
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
  harness.thread.likelySpam = true;
  harness.messages = [{
    id: 'm1',
    threadId: 'thread-1',
    from: { name: 'Lottery', email: 'win@lottery.example' },
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

describe('likely spam in the reader', () => {
  it('says so above the mail, with Not spam one click away', async () => {
    const wrapper = render();
    await flushPromises();

    const banner = wrapper.find('.thread-spam-banner');
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain('This looks like spam');
    const button = banner.findAll('button').find((candidate) => candidate.text() === 'Not spam');
    expect(button, 'the banner carries the answer').toBeDefined();
    expect(button!.attributes('aria-keyshortcuts')).toBe('Shift+I');

    await button!.trigger('click');
    expect(harness.markNotSpam).toHaveBeenCalledWith(['thread-1']);
  });

  it('stays out of the way of ordinary mail', async () => {
    harness.thread.likelySpam = false;
    const wrapper = render();
    await flushPromises();
    expect(wrapper.find('.thread-spam-banner').exists()).toBe(false);
  });
});
