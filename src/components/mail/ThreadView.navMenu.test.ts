// @vitest-environment jsdom
/*
 * The phone thread's one overflow menu: a ⋯ at the trailing edge of the nav
 * bar, holding the thread's actions and the newest message's. The bottom bar
 * has no More any more, and the reader decides which messages keep a ⋯
 * beside their sender.
 */
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h, inject } from 'vue';

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
    selectedThreadIds: [] as string[],
    messages: [] as Array<Record<string, unknown>>,
    messagesLoading: false,
    messagesError: null as string | null,
    archive: vi.fn(),
    moveToInbox: vi.fn(),
    loadMessages: vi.fn().mockResolvedValue(undefined),
    trash: vi.fn(),
    togglePinned: vi.fn(),
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
    selectedThreadIds: harness.selectedThreadIds,
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
    membershipsKnownFor: () => true, membershipsFailedFor: () => false, membershipsLoadingFor: () => false, topicIdsFor: () => [],
    hasMembershipsFor: () => false,
    byId: vi.fn(),
    loadMembershipsFor: vi.fn(),
    openPicker: vi.fn(),
  }),
}));
vi.mock('@/stores/ui', () => ({
  useUiStore: () => ({
    isMobile: true,
    isSinglePane: true,
    openOmni: vi.fn(),
    registerOverlay: () => () => {},
    dismissTopmostOverlay: vi.fn(),
  }),
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
    trash: harness.trash,
    removeFromTopic: vi.fn(),
    snoozeAt: vi.fn(),
    togglePinned: harness.togglePinned,
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
import { THREAD_MESSAGE_MENU } from './threadMessageMenu';

const run = vi.fn();

/** Stands in for a message: shows what the reader asks of it, and reports a menu when it hosts. */
const BubbleStub = defineComponent({
  props: { message: { type: Object, required: true } },
  setup(props) {
    const host = inject(THREAD_MESSAGE_MENU)!;
    const id = () => (props.message as { id: string }).id;
    if (host.hostId.value === id()) {
      host.menu.value = { canReply: true, canReplyAll: false, collapsed: false, bodyView: null, images: null, run };
    }
    return () => h('article', {
      class: 'bubble-stub',
      'data-id': id(),
      'data-host': String(host.hostId.value === id()),
      'data-inline-menu': String(host.inlineIds.value.has(id())),
    });
  },
});

function sent(id: string, minute: number) {
  return {
    id,
    threadId: 'thread-1',
    from: { name: 'Sender', email: 'sender@example.com' },
    to: [{ name: 'Me', email: 'me@example.com' }],
    subject: 'Design review',
    bodyText: 'Body',
    bodyLoaded: true,
    isDraft: false,
    isRead: true,
    timestamp: `2026-08-13T18:0${minute}:00Z`,
    attachments: [],
  };
}

function render() {
  return mount(ThreadView, {
    attachTo: document.body,
    props: { threadId: harness.thread.id, showBack: true },
    global: {
      stubs: {
        MessageBubble: BubbleStub,
        PluginSlotHost: true,
        SnoozePicker: true,
        TopicChip: true,
        Transition: { template: '<slot />' },
      },
    },
  });
}

async function settle() {
  await flushPromises();
  await vi.dynamicImportSettled();
  await flushPromises();
}

function bubbles(wrapper: ReturnType<typeof render>) {
  return wrapper.findAll('.bubble-stub').map((bubble) => ({
    id: bubble.attributes('data-id'),
    host: bubble.attributes('data-host') === 'true',
    inlineMenu: bubble.attributes('data-inline-menu') === 'true',
  }));
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.thread.workflowState = 'inbox';
  harness.selectedThreadIds = [];
  harness.messages = [sent('message-1', 0)];
  harness.messagesLoading = false;
  harness.loadMessages.mockResolvedValue(undefined);
  document.body.innerHTML = '';
});

describe('the phone thread\'s nav-bar ⋯', () => {
  it('opens from More in the bottom bar, not from a ⋯ in the nav bar', async () => {
    const wrapper = render();
    await settle();

    expect(wrapper.get('.mobile-top-bar').find('[aria-haspopup]').exists()).toBe(false);
    const more = wrapper.get('.mobile-bar [title="More actions"]');
    expect(more.attributes('aria-haspopup')).toBe('dialog');
    expect(more.attributes('aria-expanded')).toBe('false');
    wrapper.unmount();
  });

  it('opens one sheet with the thread\'s actions and the newest message\'s', async () => {
    const wrapper = render();
    await settle();

    await wrapper.get('.mobile-bar [title="More actions"]').trigger('click');
    await settle();

    expect(wrapper.get('.mobile-bar [title="More actions"]').attributes('aria-expanded')).toBe('true');
    const sheet = wrapper.get('.thread-menu');
    expect(sheet.findAll('.thread-menu__heading').map((heading) => heading.text()))
      .toEqual(['This conversation', 'Latest message']);
    expect(sheet.get('[role="group"][aria-label="This conversation"]').findAll('.menu-item').map((row) => row.text()))
      .toEqual(['Pin', 'Set aside', 'Delete']);
    const latest = sheet.get('[role="group"][aria-label="Latest message"]');
    expect(latest.text()).toContain('Show source');
    expect(latest.text()).not.toContain('Reply');

    await latest.findAll('.menu-item').find((row) => row.text() === 'Show source')!.trigger('click');
    expect(run).toHaveBeenCalledWith('source');
    expect(wrapper.find('.thread-menu').exists()).toBe(false);

    await wrapper.get('.mobile-bar [title="More actions"]').trigger('click');
    await wrapper.findAll('.thread-menu .menu-item').find((row) => row.text() === 'Delete')!.trigger('click');
    await flushPromises();
    expect(harness.trash).toHaveBeenCalledWith(['thread-1']);
    wrapper.unmount();
  });

  it('gives a single-message thread no ⋯ beside the sender', async () => {
    const wrapper = render();
    await settle();

    expect(bubbles(wrapper)).toEqual([{ id: 'message-1', host: true, inlineMenu: false }]);
    wrapper.unmount();
  });

  it('hands the newest message, not a draft, to the nav bar', async () => {
    harness.messages = [sent('message-1', 0), sent('message-2', 1), { ...sent('draft-1', 2), isDraft: true }];
    const wrapper = render();
    await settle();

    expect(bubbles(wrapper).filter((bubble) => bubble.host).map((bubble) => bubble.id)).toEqual(['message-2']);
    wrapper.unmount();
  });

  it('keeps a ⋯ beside every sender of a longer thread, the newest included', async () => {
    harness.messages = [sent('message-1', 0), sent('message-2', 1), sent('message-3', 2)];
    const wrapper = render();
    await settle();
    // The earlier messages wait behind "Show earlier".
    await wrapper.get('.thread-earlier-row').trigger('click');

    expect(bubbles(wrapper)).toEqual([
      { id: 'message-1', host: false, inlineMenu: true },
      { id: 'message-2', host: false, inlineMenu: true },
      { id: 'message-3', host: true, inlineMenu: true },
    ]);
    wrapper.unmount();
  });
});
