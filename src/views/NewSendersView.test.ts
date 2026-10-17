// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { NewSenderGroup, NewSenderStack } from '@/stores/threads';

const harness = vi.hoisted(() => ({
  push: vi.fn(),
  confirm: vi.fn(),
  acceptSender: vi.fn(),
  acceptSenders: vi.fn(),
  acceptDomain: vi.fn(),
  completeSenderReviewSetup: vi.fn(),
  setDismissedNewSender: vi.fn(),
  blockSender: vi.fn(),
  toastError: vi.fn(),
  load: vi.fn(),
  selectThread: vi.fn(),
  settings: {
    senderReviewStartedAt: null as string | null,
  },
  threads: {
    newSenderStack: null as NewSenderStack | null,
  },
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: harness.push }),
}));

vi.mock('@/stores/settings', () => ({
  useSettingsStore: () => ({
    get senderReviewStartedAt() {
      return harness.settings.senderReviewStartedAt;
    },
    acceptSender: harness.acceptSender,
    acceptSenders: harness.acceptSenders,
    acceptDomain: harness.acceptDomain,
    completeSenderReviewSetup: harness.completeSenderReviewSetup,
    setDismissedNewSender: harness.setDismissedNewSender,
  }),
}));

vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => ({
    get newSenderStack() {
      return harness.threads.newSenderStack;
    },
    load: harness.load,
    selectThread: harness.selectThread,
  }),
}));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({
    blockSender: harness.blockSender,
  }),
}));

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ error: harness.toastError, success: vi.fn() }),
}));

import NewSendersView from './NewSendersView.vue';

function makeSender(overrides: Partial<NewSenderGroup> = {}): NewSenderGroup {
  return {
    id: 'sender-1',
    senderName: 'Ada Lovelace',
    senderEmail: 'ada@example.org',
    accountId: 'account-1',
    subject: 'A message',
    previewText: 'Preview',
    count: 1,
    unreadCount: 1,
    lastMessageAt: '2026-08-31T10:00:00.000Z',
    lastMessageAtMs: Date.parse('2026-08-31T10:00:00.000Z'),
    threadIds: ['thread-1'],
    topThreadId: 'thread-1',
    spoofingText: null,
    ...overrides,
  };
}

function makeStack(senders: NewSenderGroup[]): NewSenderStack {
  return {
    id: 'new-senders',
    label: 'New senders',
    count: senders.reduce((total, sender) => total + sender.count, 0),
    unreadCount: senders.reduce((total, sender) => total + sender.unreadCount, 0),
    threadIds: senders.flatMap((sender) => sender.threadIds),
    senders,
    domainSuggestion: null,
  };
}

function render() {
  return mount(NewSendersView, {
    global: {
      stubs: {
        Avatar: true,
        DrawerButton: true,
      },
    },
  });
}

describe('NewSendersView', () => {
  beforeEach(() => {
    harness.push.mockReset();
    harness.confirm.mockReset().mockReturnValue(false);
    harness.acceptSender.mockReset();
    harness.acceptSenders.mockReset();
    harness.acceptDomain.mockReset();
    harness.completeSenderReviewSetup.mockReset();
    harness.setDismissedNewSender.mockReset();
    harness.blockSender.mockReset().mockResolvedValue(undefined);
    harness.toastError.mockReset();
    harness.load.mockReset().mockResolvedValue(undefined);
    harness.selectThread.mockReset();
    harness.settings.senderReviewStartedAt = null;
    harness.threads.newSenderStack = null;
    Object.defineProperty(window, 'confirm', {
      configurable: true,
      value: harness.confirm,
    });
  });

  it('offers sender allow for personal mailboxes without offering domain allow', async () => {
    harness.threads.newSenderStack = makeStack([
      makeSender({ senderEmail: 'person@gmail.com' }),
    ]);

    const wrapper = render();
    const article = wrapper.get('article');

    expect(article.find('.accept').exists()).toBe(true);
    expect(article.find('.domain').exists()).toBe(false);
    await article.get('.accept').trigger('click');

    expect(harness.acceptSender).toHaveBeenCalledWith('person@gmail.com');
    expect(harness.acceptDomain).not.toHaveBeenCalled();
  });

  it('offers organization domain allow and passes the normalized domain', async () => {
    harness.threads.newSenderStack = makeStack([
      makeSender({ senderEmail: 'person@Example.ORG' }),
    ]);

    const wrapper = render();
    const domainButton = wrapper.get('article .domain');

    expect(domainButton.attributes('title')).toContain('example.org');
    await domainButton.trigger('click');

    expect(harness.acceptDomain).toHaveBeenCalledWith('example.org');
  });

  it('allows all senders after review has started', async () => {
    const first = makeSender({ senderEmail: 'first@example.org' });
    const second = makeSender({
      id: 'sender-2',
      senderEmail: 'second@company.test',
      threadIds: ['thread-2'],
      topThreadId: 'thread-2',
    });
    harness.settings.senderReviewStartedAt = '2026-08-30T10:00:00.000Z';
    harness.threads.newSenderStack = makeStack([first, second]);

    const wrapper = render();
    await wrapper.get('.bulk-actions button.primary').trigger('click');

    expect(harness.acceptSenders).toHaveBeenCalledWith([
      'first@example.org',
      'second@company.test',
    ]);
    expect(harness.completeSenderReviewSetup).not.toHaveBeenCalled();
  });

  it('completes the initial sender review with all existing sender addresses', async () => {
    const first = makeSender({ senderEmail: 'first@example.org' });
    const second = makeSender({
      id: 'sender-2',
      senderEmail: 'second@company.test',
      threadIds: ['thread-2'],
      topThreadId: 'thread-2',
    });
    harness.threads.newSenderStack = makeStack([first, second]);

    const wrapper = render();
    await wrapper.get('.bulk-actions button.primary').trigger('click');

    expect(harness.completeSenderReviewSetup).toHaveBeenCalledWith([
      'first@example.org',
      'second@company.test',
    ]);
    expect(harness.acceptSenders).not.toHaveBeenCalled();
  });

  it('blocks only after confirmation, then blocks remotely, dismisses, and reloads', async () => {
    const sender = makeSender({
      senderEmail: 'blocked@example.org',
      accountId: 'account-9',
      lastMessageAt: '2026-08-31T11:00:00.000Z',
      lastMessageAtMs: Date.parse('2026-08-31T11:00:00.000Z'),
    });
    harness.threads.newSenderStack = makeStack([sender]);

    const wrapper = render();
    const blockButton = wrapper.get('article .actions button:last-child');
    await blockButton.trigger('click');

    expect(harness.confirm).toHaveBeenCalledTimes(1);
    expect(harness.setDismissedNewSender).not.toHaveBeenCalled();
    expect(harness.blockSender).not.toHaveBeenCalled();
    expect(harness.load).not.toHaveBeenCalled();

    harness.confirm.mockReturnValue(true);
    await blockButton.trigger('click');
    await flushPromises();

    expect(harness.setDismissedNewSender).toHaveBeenCalledWith(
      'blocked@example.org',
      '2026-08-31T11:00:00.000Z',
    );
    expect(harness.blockSender).toHaveBeenCalledWith({
      pattern: 'blocked@example.org',
      accountId: 'account-9',
    });
    // The block has to be durable before the sender leaves the review list.
    expect(harness.blockSender.mock.invocationCallOrder[0]).toBeLessThan(
      harness.setDismissedNewSender.mock.invocationCallOrder[0]!,
    );
    expect(harness.load).toHaveBeenCalledTimes(1);
    expect(harness.toastError).not.toHaveBeenCalled();
  });

  it('keeps the sender in the review list when the block fails', async () => {
    const sender = makeSender({ senderEmail: 'blocked@example.org', accountId: 'account-9' });
    harness.threads.newSenderStack = makeStack([sender]);
    harness.confirm.mockReturnValue(true);
    harness.blockSender.mockRejectedValue(new Error('rules table locked'));

    const wrapper = render();
    await wrapper.get('article .actions button:last-child').trigger('click');
    await flushPromises();

    expect(harness.setDismissedNewSender).not.toHaveBeenCalled();
    expect(harness.load).not.toHaveBeenCalled();
    expect(harness.toastError).toHaveBeenCalledWith(
      'Could not block blocked@example.org. The sender is still in the review list.',
    );
  });

  it('selects the sender thread and returns to the inbox when its copy is clicked', async () => {
    harness.threads.newSenderStack = makeStack([
      makeSender({ topThreadId: 'thread-to-open' }),
    ]);

    const wrapper = render();
    await wrapper.get('article .sender-copy').trigger('click');

    expect(harness.selectThread).toHaveBeenCalledWith('thread-to-open');
    expect(harness.push).toHaveBeenCalledWith({ name: 'inbox' });
  });
});
