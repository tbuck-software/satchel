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
    blockSender: vi.fn().mockResolvedValue(undefined),
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
  useSettingsStore: () => ({ readerZoom: 100, setReaderZoom: vi.fn() }),
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

import { useConfirm } from '@/composables/useConfirm';
import { activeThreadReaderCommands } from '@/composables/useThreadReaderCommands';
import ThreadView from './ThreadView.vue';

function message(id: string, email: string, extra: Record<string, unknown> = {}) {
  return {
    id,
    threadId: 'thread-1',
    from: { name: email, email },
    to: [],
    subject: 'Design review',
    bodyText: '',
    bodyLoaded: true,
    isDraft: false,
    isRead: true,
    timestamp: '2026-08-13T18:00:00Z',
    attachments: [],
    ...extra,
  };
}

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

async function blockAndAnswer(scope: 'address' | 'domain', answer: boolean) {
  const confirm = useConfirm();
  const blocking = activeThreadReaderCommands()!.blockSender(scope) as Promise<void>;
  await flushPromises();
  const question = confirm.state.pending;
  if (question) confirm.answer(answer);
  await blocking;
  return question;
}

beforeEach(() => {
  vi.clearAllMocks();
  useConfirm().answer(false);
  harness.messages = [];
});

describe('block sender', () => {
  it('blocks the latest foreign sender, not our own reply or a draft', async () => {
    harness.messages = [
      message('m1', 'Sender@Example.org'),
      message('m2', 'me@example.com'),
      message('m3', 'me@example.com', { isDraft: true }),
    ];
    render();
    await flushPromises();

    const question = await blockAndAnswer('address', true);

    expect(question).toMatchObject({ tone: 'danger', confirmLabel: 'Block' });
    expect(question?.body).toContain('Sender@Example.org');
    expect(harness.blockSender).toHaveBeenCalledWith({
      pattern: 'Sender@Example.org',
      accountId: 'account-1',
      threadIds: ['thread-1'],
    });
  });

  it('does nothing when the dialog is dismissed', async () => {
    harness.messages = [message('m1', 'sender@example.org')];
    render();
    await flushPromises();

    await blockAndAnswer('domain', false);

    expect(harness.blockSender).not.toHaveBeenCalled();
  });

  it('refuses to block our own domain', async () => {
    harness.messages = [message('m1', 'colleague@example.com')];
    render();
    await flushPromises();

    const question = await blockAndAnswer('domain', true);

    expect(question).toBeNull();
    expect(harness.blockSender).not.toHaveBeenCalled();
    expect(harness.toastError).toHaveBeenCalledOnce();
  });

  it('refuses when the thread holds only our own mail', async () => {
    harness.messages = [message('m1', 'me@example.com')];
    render();
    await flushPromises();

    const question = await blockAndAnswer('address', true);

    expect(question).toBeNull();
    expect(harness.blockSender).not.toHaveBeenCalled();
  });
});
