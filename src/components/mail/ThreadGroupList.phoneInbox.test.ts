// @vitest-environment jsdom
import { shallowMount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import type { NewSenderStack } from '@/stores/threads';
import type { ThreadListEntry } from '@/utils/threadList';
import ThreadGroupList from './ThreadGroupList.vue';

/*
 * The phone Inbox: New senders is a plain row, Priority shows two mails and
 * one full-width row to the rest. Both rows are thumb targets, and their
 * heights reach the virtualiser through the same constants the markup uses.
 */

function thread(id: string) {
  return {
    id,
    accountId: 'account-1',
    subject: `Subject ${id}`,
    participants: [{ name: 'Sender', email: 'sender@example.com' }],
    snippet: 'Preview',
    lastMessageAt: '2026-08-06T12:00:00Z',
    lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox' as const,
    priority: 3 as const,
    mailboxNames: ['INBOX'],
    tags: [],
  };
}

const stack: NewSenderStack = {
  id: 'new-sender-stack',
  label: 'New senders',
  count: 15,
  unreadCount: 15,
  threadIds: ['n1'],
  senders: [],
  domainSuggestion: null,
};

const ITEMS = [
  { kind: 'new-sender-stack', id: 'new-sender-stack', stack },
  {
    kind: 'priority-card',
    id: 'priority-card',
    bucket: {
      id: 'bucket-priority',
      key: 'priority',
      label: 'Priority',
      tone: 'priority',
      count: 6,
      unreadCount: 2,
      threadIds: ['p1', 'p2'],
      previewText: '',
      previewItems: [],
      threads: [],
    },
    threads: [thread('p1'), thread('p2')],
    hiddenCount: 4,
  },
] as unknown as ThreadListEntry[];

function mountList(phoneLayout: boolean) {
  return shallowMount(ThreadGroupList, {
    props: {
      items: ITEMS,
      selectedThreadIds: [],
      senderReviewSetupComplete: true,
      newSendersExpanded: false,
      bulkBusy: false,
      phoneLayout,
    },
    global: { stubs: { CardButton: false, AppButton: false, AppIcon: false } },
  });
}

const heightOf = (wrapper: ReturnType<typeof mountList>, label: string) =>
  wrapper.get(`section[aria-label="${label}"]`).attributes('style');

describe('phone Inbox list', () => {
  it('draws New senders as a plain row with Review as its own target', async () => {
    const wrapper = mountList(true);

    // The card's surface and padding come off under the phone modifier.
    expect(wrapper.get('.thread-list-scroller').classes()).toContain('thread-list--phone');
    // One 48px row, no padding around it.
    expect(heightOf(wrapper, 'New senders')).toContain('height: 48px');
    expect(wrapper.get('.thread-list-scroller').attributes('style')).toContain('--review-banner-h: 48px');

    const review = wrapper.get('.review-card__open');
    expect(review.text()).toBe('Review');
    await review.trigger('click');
    expect(wrapper.emitted('openNewSenders')).toHaveLength(1);

    await wrapper.get('[data-list-target-key="new-senders:new-sender-stack"]').trigger('click');
    expect(wrapper.emitted('toggleNewSenders')).toHaveLength(1);
  });

  it('shows two priority mails and a 44px row to the rest', async () => {
    const wrapper = mountList(true);

    // Padding, header, two rows, the "more" row; header and row read the
    // section's own header height.
    const style = heightOf(wrapper, 'Priority');
    expect(style).toContain(`height: ${12 + 44 + 2 * 72 + 44}px`);
    expect(style).toContain('--card-header-h: 44px');
    expect(wrapper.get('.priority-group__header').classes()).toContain('h-[var(--card-header-h)]');

    const more = wrapper.get('[data-list-target-key="bucket:bucket-priority"]');
    expect(more.text()).toBe('4 more priority emails');
    await more.trigger('click');
    expect(wrapper.emitted('openBucket')).toEqual([['bucket-priority']]);
  });

  it('leaves the desktop card, header and caption as they were', () => {
    const wrapper = mountList(false);

    expect(wrapper.get('.thread-list-scroller').classes()).not.toContain('thread-list--phone');
    expect(heightOf(wrapper, 'New senders')).toContain('height: 60px');
    expect(heightOf(wrapper, 'Priority')).toContain(`height: ${12 + 32 + 2 * 72 + 32}px`);
    expect(wrapper.get('[data-list-target-key="bucket:bucket-priority"]').text()).toBe('+4 more emails');
  });
});
