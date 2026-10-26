// @vitest-environment jsdom
import { shallowMount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ThreadGroupList from './ThreadGroupList.vue';
import type { NewSenderGroup, NewSenderStack } from '@/stores/threads';

function sender(index: number): NewSenderGroup {
  return {
    id: `new-sender-${index}`,
    senderName: `Sender ${index}`,
    senderEmail: `sender${index}@example.com`,
    accountId: 'account-1',
    subject: `Subject ${index}`,
    previewText: '',
    count: 1,
    unreadCount: 1,
    lastMessageAt: '2026-10-26T12:00:00Z',
    lastMessageAtMs: Date.parse('2026-10-26T12:00:00Z'),
    threadIds: [`thread-${index}`],
    topThreadId: `thread-${index}`,
    spoofingText: null,
  };
}

function stack(count: number): NewSenderStack {
  const senders = Array.from({ length: count }, (_, index) => sender(index + 1));
  return {
    id: 'new-sender-stack',
    label: 'New senders',
    count,
    unreadCount: count,
    threadIds: senders.flatMap((entry) => entry.threadIds),
    senders,
    domainSuggestion: { domain: 'example.com', senderCount: 2, threadIds: ['thread-1', 'thread-2'] },
  };
}

function mountReview(options: { expanded: boolean; setupComplete?: boolean; count?: number }) {
  return shallowMount(ThreadGroupList, {
    props: {
      items: [{ kind: 'new-sender-stack', id: 'new-sender-stack', stack: stack(options.count ?? 5) }],
      selectedThreadIds: [],
      senderReviewSetupComplete: options.setupComplete ?? false,
      newSendersExpanded: options.expanded,
      bulkBusy: false,
    },
    global: { stubs: { CardButton: false, AppButton: false } },
  });
}

const sectionHeight = (wrapper: ReturnType<typeof mountReview>) =>
  wrapper.get('section[aria-label="New senders"]').attributes('style');

describe('new-sender review in the list', () => {
  it('collapses to one line that names the senders and offers the review', async () => {
    const wrapper = mountReview({ expanded: false });

    const toggle = wrapper.get('[data-list-target-key="new-senders:new-sender-stack"]');
    expect(toggle.attributes('aria-expanded')).toBe('false');
    expect(toggle.text()).toContain('Sender 1, Sender 2, Sender 3');
    expect(wrapper.findAll('[data-list-target-key^="new-sender:"]')).toHaveLength(0);
    // Section padding plus the banner, nothing else.
    expect(sectionHeight(wrapper)).toContain('height: 60px');

    await toggle.trigger('click');
    expect(wrapper.emitted('toggleNewSenders')).toHaveLength(1);

    const review = wrapper.findAll('button').find((button) => button.text() === 'Review');
    await review!.trigger('click');
    expect(wrapper.emitted('openNewSenders')).toHaveLength(1);
  });

  it('expands to the newest senders with their actions and both footer lines', async () => {
    const wrapper = mountReview({ expanded: true });

    expect(wrapper.get('[data-list-target-key="new-senders:new-sender-stack"]').attributes('aria-expanded')).toBe('true');
    const rows = wrapper.findAll('[data-list-target-key^="new-sender:"]');
    expect(rows.map((row) => row.attributes('data-list-target-key'))).toEqual([
      'new-sender:new-sender-1',
      'new-sender:new-sender-2',
      'new-sender:new-sender-3',
    ]);
    expect(rows[1]!.text()).toContain('sender2@example.com');
    // Banner, three rows, domain line, rest line, card padding.
    expect(sectionHeight(wrapper)).toContain(`height: ${16 + 44 + 3 * 72 + 32 + 32 + 4}px`);

    const allow = rows[1]!.findAll('button').find((button) => button.text() === 'Allow');
    await allow!.trigger('click');
    expect(wrapper.emitted('newSenderAction')).toEqual([[{ sender: sender(2), action: 'accept' }]]);
    expect(wrapper.emitted('activate')).toBeUndefined();

    expect(wrapper.text()).toContain('+2 more senders');
    const allowAll = wrapper.findAll('button').find((button) => button.text() === 'Allow all 5 senders');
    await allowAll!.trigger('click');
    expect(wrapper.emitted('acceptExistingSenders')).toHaveLength(1);
  });

  it('drops the rest line when every sender is shown and setup is done', () => {
    const wrapper = mountReview({ expanded: true, setupComplete: true, count: 2 });

    expect(wrapper.findAll('[data-list-target-key^="new-sender:"]')).toHaveLength(2);
    expect(wrapper.text()).not.toContain('more sender');
    expect(sectionHeight(wrapper)).toContain(`height: ${16 + 44 + 2 * 72 + 32 + 4}px`);
  });
});
