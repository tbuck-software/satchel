// @vitest-environment jsdom
import { defineComponent, nextTick } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '@/types/mail';

const harness = vi.hoisted(() => ({
  messages: [] as Message[],
  messagesError: null as string | null,
  openDraft: vi.fn(),
  loadMessages: vi.fn().mockResolvedValue(undefined),
  loadMessageDetails: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@/stores/connection', () => ({
  useConnectionStore: () => ({ backendErrorFor: () => harness.messagesError }),
}));
vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => ({ accounts: [], capabilitiesFor: () => ({ can_reply: true }) }),
}));
vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({
    getByThread: () => harness.messages,
    isLoading: () => false,
    isBodyLoading: () => false,
    loadByThread: harness.loadMessages,
    openThread: harness.loadMessages,
    hasThread: () => false,
    retainThread: () => () => {},
    needsDetails: () => false,
    loadAttachmentsForMessages: harness.loadMessageDetails,
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
    threads: [{
      id: 'thread-1', accountId: 'account-1', subject: 'Navigation', mailboxNames: ['Inbox'],
      unreadCount: 0, isStarred: false, isMuted: false, tags: [], workflowState: 'inbox',
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
    hasMembershipsFor: () => false, byId: vi.fn(), loadMembershipsFor: vi.fn(), openPicker: vi.fn(),
  }),
}));
vi.mock('@/stores/ui', () => ({
  useUiStore: () => ({ isMobile: false, openOmni: vi.fn() }),
}));
vi.mock('@/stores/composer', () => ({
  useComposerStore: () => ({
    openReply: vi.fn(), openForward: vi.fn(), openResend: vi.fn(), openDraft: harness.openDraft,
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
    archive: vi.fn(), trash: vi.fn(), removeFromTopic: vi.fn(), snoozeAt: vi.fn(),
    togglePinned: vi.fn(), setAside: vi.fn(), moveToInbox: vi.fn(),
  }),
}));
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), info: vi.fn(), error: vi.fn() }),
}));
vi.mock('@/services/backend', () => ({
  backendBlockSender: vi.fn(), backendDownloadAttachment: vi.fn(),
  backendMoveThreadsToFolder: vi.fn(), backendSupportsNativeCommands: vi.fn(() => true),
}));
vi.mock('@/utils/perf', () => ({ perfStart: () => undefined }));
vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }));

import ThreadView from './ThreadView.vue';
import {
  moveThreadMessageFocus,
  openThreadMessageFocus,
  toggleAllThreadMessages,
  toggleFocusedThreadMessage,
} from '@/composables/useThreadMessageKeyboardNavigation';

const MessageBubbleStub = defineComponent({
  props: {
    message: { type: Object, required: true },
    collapsed: Boolean,
    keyboardFocused: Boolean,
    bodyMounted: { type: Boolean, default: true },
  },
  template: `
    <article
      :data-thread-message-id="message.id"
      :data-collapsed="String(collapsed)"
      :data-keyboard-focused="String(keyboardFocused)"
      :data-body-mounted="String(bodyMounted && !collapsed)"
      tabindex="-1"
    />
  `,
});

/* The draft card does not fold; it marks focus like a message does. */
const DraftCardStub = defineComponent({
  props: {
    message: { type: Object, required: true },
    keyboardFocused: Boolean,
  },
  template: `
    <article
      :data-thread-message-id="message.id"
      data-draft="true"
      :data-keyboard-focused="String(keyboardFocused)"
      tabindex="-1"
    />
  `,
});

/** Stands in for the browser's observer; the test decides what is on screen. */
class FakeIntersectionObserver {
  static current: FakeIntersectionObserver | null = null;
  readonly targets = new Set<Element>();
  constructor(private readonly callback: IntersectionObserverCallback) {
    FakeIntersectionObserver.current = this;
  }
  observe(target: Element) { this.targets.add(target); }
  unobserve(target: Element) { this.targets.delete(target); }
  disconnect() { this.targets.clear(); }
  takeRecords() { return []; }
  report(visibleIds: string[]) {
    const entries = [...this.targets].map((target) => ({
      target,
      isIntersecting: visibleIds.includes((target as HTMLElement).dataset.threadMessageId ?? ''),
    })) as unknown as IntersectionObserverEntry[];
    this.callback(entries, this as unknown as IntersectionObserver);
  }
}

function message(id: string, options: { unread?: boolean; draft?: boolean } = {}): Message {
  return {
    id,
    threadId: 'thread-1',
    from: { name: options.draft ? 'Ich' : 'Lena', email: options.draft ? 'me@example.com' : 'lena@example.com' },
    to: [],
    subject: 'Navigation',
    bodyText: id,
    timestamp: '2026-08-13T00:00:00Z',
    isRead: !options.unread,
    isDraft: options.draft,
    attachments: [],
  };
}

function render() {
  return mount(ThreadView, {
    props: { threadId: 'thread-1', showBack: true },
    global: {
      stubs: {
        MessageBubble: MessageBubbleStub,
        ThreadDraftCard: DraftCardStub,
        ThreadToolbar: true,
        ThreadPaneSkeleton: true,
        PluginSlotHost: true,
        SnoozePicker: true,
        TopicChip: true,
      },
    },
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.messages = [
    message('first'),
    message('unread', { unread: true }),
    message('latest'),
    message('draft', { draft: true }),
  ];
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  document.body.innerHTML = '';
});

describe('thread message keyboard navigation', () => {
  it('reveals the earlier messages folded and opens only what is toggled', async () => {
    const wrapper = render();
    await flushPromises();

    expect(wrapper.findAll('[data-thread-message-id]')).toHaveLength(2);
    expect(moveThreadMessageFocus(-1)).toBe(true);
    await nextTick();

    expect(wrapper.findAll('[data-thread-message-id]')).toHaveLength(4);
    expect(wrapper.get('[data-thread-message-id="first"]').attributes('data-keyboard-focused')).toBe('true');
    // Moving does not open: the earlier messages arrive folded.
    expect(wrapper.get('[data-thread-message-id="first"]').attributes('data-collapsed')).toBe('true');
    expect(wrapper.get('[data-thread-message-id="unread"]').attributes('data-collapsed')).toBe('true');
    expect(wrapper.get('[data-thread-message-id="latest"]').attributes('data-collapsed')).toBe('false');

    expect(toggleFocusedThreadMessage()).toBe(true);
    await nextTick();
    expect(wrapper.get('[data-thread-message-id="first"]').attributes('data-collapsed')).toBe('false');

    // Every message folds; the draft card has no folded form.
    expect(toggleAllThreadMessages()).toBe(true);
    await nextTick();
    expect(wrapper.findAll('[data-collapsed="true"]')).toHaveLength(3);
    expect(wrapper.find('[data-draft="true"]').exists()).toBe(true);

    expect(toggleAllThreadMessages()).toBe(true);
    await nextTick();
    expect(wrapper.findAll('[data-collapsed="true"]')).toHaveLength(0);

    window.dispatchEvent(new Event('pointermove'));
    await nextTick();
    expect(wrapper.findAll('[data-keyboard-focused="true"]')).toHaveLength(0);
    wrapper.unmount();
  });

  it('treats the draft as the final focus target and opens it with Enter semantics', async () => {
    const wrapper = render();
    await flushPromises();

    moveThreadMessageFocus(1);
    moveThreadMessageFocus(1);
    await nextTick();
    expect(wrapper.get('[data-thread-message-id="draft"]').attributes('data-keyboard-focused')).toBe('true');

    expect(openThreadMessageFocus()).toBe(true);
    expect(harness.openDraft).toHaveBeenCalledWith('draft');
    // Toggling a draft goes back into it too: there is nothing to fold.
    expect(toggleFocusedThreadMessage()).toBe(true);
    expect(harness.openDraft).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it('loads a body for the message it opens, not for the ones it passes', async () => {
    harness.messages = [
      { ...message('first'), bodyLoaded: false },
      { ...message('middle'), bodyLoaded: false },
      { ...message('latest'), bodyLoaded: true },
    ];
    const wrapper = render();
    await flushPromises();
    harness.loadMessageDetails.mockClear();

    expect(moveThreadMessageFocus(-1)).toBe(true);
    expect(moveThreadMessageFocus(-1)).toBe(true);
    await flushPromises();
    expect(harness.loadMessageDetails).not.toHaveBeenCalled();

    expect(toggleFocusedThreadMessage()).toBe(true);
    await flushPromises();
    expect(harness.loadMessageDetails).toHaveBeenCalledTimes(1);
    expect(harness.loadMessageDetails).toHaveBeenCalledWith('thread-1', ['first']);
    wrapper.unmount();
  });

  it('mounts renderers only for open messages near the viewport', async () => {
    const previous = globalThis.IntersectionObserver;
    globalThis.IntersectionObserver = FakeIntersectionObserver as unknown as typeof IntersectionObserver;
    try {
      harness.messages = ['m1', 'm2', 'm3', 'm4', 'm5'].map((id) => message(id));
      const wrapper = render();
      await flushPromises();

      // Folds the open latest message, then opens all five.
      expect(toggleAllThreadMessages()).toBe(true);
      expect(toggleAllThreadMessages()).toBe(true);
      await flushPromises();
      expect(wrapper.findAll('[data-collapsed="false"]')).toHaveLength(5);
      // Opening all of them mounts only the focused one until the observer speaks.
      expect(wrapper.findAll('[data-body-mounted="true"]').map((row) => row.attributes('data-thread-message-id')))
        .toEqual(['m5']);

      FakeIntersectionObserver.current!.report(['m1', 'm2']);
      await nextTick();
      expect(wrapper.findAll('[data-body-mounted="true"]').map((row) => row.attributes('data-thread-message-id')))
        .toEqual(['m1', 'm2', 'm5']);
      wrapper.unmount();
    } finally {
      globalThis.IntersectionObserver = previous;
      FakeIntersectionObserver.current = null;
    }
  });
});
