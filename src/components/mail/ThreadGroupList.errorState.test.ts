// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import type { Thread } from '@/types/mail';
import ThreadGroupList from './ThreadGroupList.vue';

function thread(): Thread {
  return {
    id: 'thread-1',
    accountId: 'account-1',
    subject: 'Subject',
    participants: [{ name: 'Sender', email: 'sender@example.com' }],
    snippet: 'Preview',
    lastMessageAt: '2026-09-06T12:00:00Z',
    lastMessageAtMs: Date.parse('2026-09-06T12:00:00Z'),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 1,
    mailboxNames: ['INBOX'],
    tags: [],
  } as Thread;
}

function render(props: Record<string, unknown>) {
  return mount(ThreadGroupList, {
    props: {
      items: [],
      selectedThreadIds: [],
      senderReviewSetupComplete: true,
      bulkBusy: false,
      ...props,
    },
    global: { stubs: { ThreadListItem: true } },
  });
}

describe('thread list load failure', () => {
  it('names the failure instead of claiming the view is empty', () => {
    const wrapper = render({ error: 'backend.errors.threadPersistenceFailed' });

    const alert = wrapper.get('[role="alert"]');
    expect(alert.text()).toContain('Your mail could not be loaded.');
    expect(alert.text()).toContain('The thread data could not be accessed. Please try again.');
    expect(wrapper.text()).not.toContain('No threads in this view.');
  });

  it('shows the empty view only when nothing failed', () => {
    const wrapper = render({ error: null });

    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(wrapper.text()).toContain('No threads in this view.');
  });

  it('asks for exactly one retry per click', async () => {
    const wrapper = render({ error: 'db is locked' });

    await wrapper.get('[role="alert"] button').trigger('click');

    expect(wrapper.emitted('retry')).toHaveLength(1);
  });

  it('says the mailbox is still arriving instead of calling it empty', () => {
    const wrapper = render({ error: null, backfill: { remaining: 400, total: 1000 } });

    expect(wrapper.get('[role="status"]').text()).toContain('Loading your mailbox — 600 of 1000');
    expect(wrapper.text()).not.toContain('No threads in this view.');
  });

  it('counts down without a total, rather than inventing one', () => {
    const wrapper = render({ backfill: { remaining: 400, total: 0 } });

    expect(wrapper.get('[role="status"]').text()).toContain('Loading your mailbox — 400 left');
  });

  it('lets the load failure win over the backfill hint', () => {
    const wrapper = render({ error: 'db is locked', backfill: { remaining: 400, total: 1000 } });

    expect(wrapper.find('[role="alert"]').exists()).toBe(true);
    expect(wrapper.text()).not.toContain('Loading your mailbox');
  });

  it('leaves a published snapshot on screen — that failure is the banner alone', () => {
    const wrapper = render({
      error: 'db is locked',
      items: [{ kind: 'thread', id: 'row-thread-1', thread: thread() }],
    });

    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(wrapper.find('thread-list-item-stub').exists()).toBe(true);
  });

  it('ends a list whose next page failed with a retry, not as if it were complete', async () => {
    const wrapper = render({
      loadMoreFailed: true,
      items: [{ kind: 'thread', id: 'row-thread-1', thread: thread() }],
    });

    const alert = wrapper.get('[role="alert"]');
    expect(alert.text()).toContain('Older mail could not be loaded');
    await alert.get('button').trigger('click');
    expect(wrapper.emitted('retryLoadMore')).toHaveLength(1);
  });
});
