// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '@/types/mail';

const harness = vi.hoisted(() => ({
  messages: [] as Message[],
  openDraft: vi.fn(),
  openReply: vi.fn(),
  isMobile: false,
  refreshToastClearance: vi.fn(),
}));

vi.mock('@/stores/connection', () => ({
  useConnectionStore: () => ({ backendErrorFor: () => null }),
}));
vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => ({
    accounts: [{ id: 'account-1', email: 'me@example.com', displayName: 'Work', color: null }],
    capabilitiesFor: () => ({ can_reply: true }),
  }),
}));
vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({
    getByThread: () => harness.messages,
    isLoading: () => false,
    isBodyLoading: () => false,
    loadByThread: vi.fn().mockResolvedValue(undefined),
    openThread: vi.fn().mockResolvedValue(undefined),
    hasThread: () => true,
    retainThread: () => () => {},
    needsDetails: () => false,
    loadAttachmentsForMessages: vi.fn().mockResolvedValue(undefined),
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
      id: 'thread-1', accountId: 'account-1', subject: 'Staging', mailboxNames: ['Inbox'],
      unreadCount: 0, isStarred: false, isMuted: false, tags: [], workflowState: 'inbox',
    }],
    find(this: { threads: Array<{ id: string }> }, id: string) {
      return this.threads.find((thread) => thread.id === id);
    },
    scopedVisibleThreads: [],
    focusNeighborThread: vi.fn(),
    load: vi.fn(),
  }),
}));
vi.mock('@/stores/topics', () => ({
  useTopicsStore: () => ({
    topicIdsFor: () => [],
    hasMembershipsFor: () => true, byId: vi.fn(), loadMembershipsFor: vi.fn(), openPicker: vi.fn(),
  }),
}));
vi.mock('@/stores/ui', () => ({
  useUiStore: () => ({ isMobile: harness.isMobile, openOmni: vi.fn() }),
}));
vi.mock('@/stores/composer', () => ({
  useComposerStore: () => ({
    openReply: harness.openReply, openForward: vi.fn(), openResend: vi.fn(), openDraft: harness.openDraft,
  }),
}));
vi.mock('@/composables/useDraftActions', () => ({
  useDraftActions: () => ({ discard: vi.fn(), isBusy: () => false }),
}));
vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({ setRead: vi.fn().mockResolvedValue(true) }),
}));
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), info: vi.fn(), error: vi.fn() }),
}));
vi.mock('@/composables/useToastClearance', () => ({
  useToastClearance: vi.fn(),
  refreshToastClearance: harness.refreshToastClearance,
}));
vi.mock('@/utils/perf', () => ({ perfStart: () => undefined }));
vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }));

import ThreadView from './ThreadView.vue';

function message(id: string, overrides: Partial<Message> = {}): Message {
  return {
    id,
    threadId: 'thread-1',
    from: { name: 'Sarah Kim', email: 'sarah@example.com' },
    to: [{ name: 'Work', email: 'me@example.com' }],
    subject: 'Staging',
    bodyText: id,
    timestamp: '2026-08-13T00:00:00Z',
    isRead: true,
    attachments: [],
    ...overrides,
  };
}

const draft = message('draft-1', {
  isDraft: true,
  from: { name: 'Work', email: 'me@example.com' },
  // Saved drafts often carry the address as the name.
  to: [{ name: 'sarah@example.com', email: 'sarah@example.com' }],
});

function render() {
  return mount(ThreadView, {
    props: { threadId: 'thread-1' },
    global: {
      stubs: {
        MessageBubble: true,
        ThreadToolbar: true,
        ThreadPaneSkeleton: true,
        ThreadMobileActions: true,
        PluginSlotHost: true,
        SnoozePicker: true,
        TopicChip: true,
      },
    },
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.isMobile = false;
  harness.messages = [message('first')];
});

afterEach(() => {
  document.body.innerHTML = '';
});

describe('thread reply foot', () => {
  it('replies, and takes the toolbar reply in its place', async () => {
    const wrapper = render();
    await flushPromises();

    const button = wrapper.get('.reply-bar').findAll('button').at(-1)!;
    expect(button.text()).toBe('Reply');
    expect(wrapper.get('.reply-bar').text()).toContain('Reply to Sarah Kim');
    expect(wrapper.findComponent({ name: 'ThreadToolbar' }).props('replyInFoot')).toBe(true);

    await button.trigger('click');
    expect(harness.openReply).toHaveBeenCalledWith('thread-1');
    expect(harness.openDraft).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('resumes the unsent draft instead of starting a second reply', async () => {
    harness.messages = [message('first'), draft];
    const wrapper = render();
    await flushPromises();

    const bar = wrapper.get('.reply-bar');
    const button = bar.findAll('button').at(-1)!;
    expect(button.text()).toBe('Resume draft');
    // The thread knows the addressee by name even when the draft does not.
    expect(bar.text()).toContain('Continue your reply to Sarah Kim');

    await button.trigger('click');
    await bar.findAll('button')[0]!.trigger('click');
    expect(harness.openDraft).toHaveBeenCalledTimes(2);
    expect(harness.openDraft).toHaveBeenCalledWith('draft-1');
    expect(harness.openReply).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  /*
   * In a short thread the foot sits right under the last message, so a body
   * arriving or a message folding moves it. The toast stack measures where
   * the bar is only when told; without this it stayed where the bar was.
   */
  it('tells the toast stack when the thread above the foot changes height', async () => {
    const observers: Array<{ callback: ResizeObserverCallback; targets: Element[] }> = [];
    vi.stubGlobal('ResizeObserver', class {
      private entry: { callback: ResizeObserverCallback; targets: Element[] };
      constructor(callback: ResizeObserverCallback) {
        this.entry = { callback, targets: [] };
        observers.push(this.entry);
      }
      observe(target: Element) { this.entry.targets.push(target); }
      unobserve() {}
      disconnect() { this.entry.targets = []; }
    });
    try {
      const wrapper = render();
      await flushPromises();

      const column = wrapper.get('.thread-reading-column').element;
      const footObserver = observers.find((entry) => entry.targets.includes(column));
      expect(footObserver?.targets).toContain(wrapper.get('.thread-reply-foot').element);

      harness.refreshToastClearance.mockClear();
      footObserver!.callback([], {} as ResizeObserver);
      expect(harness.refreshToastClearance).toHaveBeenCalledTimes(1);
      wrapper.unmount();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('leaves the phone to its action bar', async () => {
    harness.isMobile = true;
    const wrapper = render();
    await flushPromises();

    expect(wrapper.find('.reply-bar').exists()).toBe(false);
    wrapper.unmount();
  });
});
