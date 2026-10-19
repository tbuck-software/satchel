// @vitest-environment jsdom
import { shallowMount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ThreadGroupList from './ThreadGroupList.vue';
import ThreadListItem from './ThreadListItem.vue';
import TopicsFolderCard from '@/components/topics/TopicsFolderCard.vue';

describe('thread group list action context', () => {
  it('passes only each row selection state', () => {
    const thread = {
      id: 'thread-1',
      accountId: 'account-1',
      subject: 'Betreff',
      participants: [{ name: 'Absender', email: 'sender@example.com' }],
      snippet: 'Vorschau',
      lastMessageAt: '2026-08-06T12:00:00Z',
      lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
      messageCount: 1,
      unreadCount: 0,
      hasAttachments: false,
      workflowState: 'inbox' as const,
      priority: 1 as const,
      mailboxNames: ['INBOX'],
      tags: [],
    };
    const wrapper = shallowMount(ThreadGroupList, {
      props: {
        items: [{ kind: 'thread', id: 'row-thread-1', thread }],
        selectedThreadIds: ['thread-1', 'thread-2'],
        senderReviewSetupComplete: true,
        bulkBusy: false,
      },
    });

    expect(wrapper.getComponent(ThreadListItem).props('selected')).toBe(true);
    expect(wrapper.getComponent(ThreadListItem).props()).not.toHaveProperty('selectedIds');
  });

  it('identifies the exact date group when all its threads are checked off', async () => {
    const wrapper = shallowMount(ThreadGroupList, {
      props: {
        items: [{
          kind: 'date-header',
          id: 'date-month-2026-08',
          label: 'August',
          count: 2,
          threadIds: ['thread-1', 'thread-2'],
        }],
        selectedThreadIds: [],
        senderReviewSetupComplete: true,
        bulkBusy: false,
      },
      global: {
        stubs: { SectionHeading: false },
      },
    });

    await wrapper.get('[aria-label="Check off every email in “August”"]').trigger('click');

    expect(wrapper.emitted('dateGroupDone')).toEqual([[
      { groupId: 'date-month-2026-08', threadIds: ['thread-1', 'thread-2'] },
    ]]);
  });

  it('renders the priority card rows and opens the bucket from its footer', async () => {
    const prioThread = (id: string) => ({
      id,
      accountId: 'account-1',
      subject: 'Wichtig',
      participants: [{ name: 'Absender', email: 'sender@example.com' }],
      snippet: 'Vorschau',
      lastMessageAt: '2026-08-24T09:00:00Z',
      lastMessageAtMs: Date.parse('2026-08-24T09:00:00Z'),
      messageCount: 1,
      unreadCount: 1,
      hasAttachments: false,
      workflowState: 'inbox' as const,
      priority: 2 as const,
      mailboxNames: ['INBOX'],
      tags: [],
    });
    const wrapper = shallowMount(ThreadGroupList, {
      props: {
        items: [{
          kind: 'priority-card',
          id: 'priority-card',
          bucket: {
            id: 'bucket-priority',
            key: 'priority',
            label: 'Priorität',
            tone: 'priority',
            count: 5,
            unreadCount: 4,
            threadIds: ['p1', 'p2', 'p3'],
            previewText: '',
            previewItems: [],
          },
          threads: [prioThread('p1'), prioThread('p2'), prioThread('p3')],
          hiddenCount: 2,
        }],
        selectedThreadIds: [],
        senderReviewSetupComplete: true,
        bulkBusy: false,
      },
      global: { stubs: { CardButton: false } },
    });

    expect(wrapper.findAllComponents(ThreadListItem)).toHaveLength(3);

    const footer = wrapper.get('[data-list-target-key="bucket:bucket-priority"]');
    expect(footer.text()).toContain('+2 more emails');
    await footer.trigger('click');
    expect(wrapper.emitted('openBucket')).toEqual([['bucket-priority']]);
  });

  it('keeps an empty topics folder out of the smart folder section', () => {
    const folderRow = (id: string) => ({
      kind: 'folder-row' as const,
      id,
      folders: [],
    });

    const withoutTopics = shallowMount(ThreadGroupList, {
      props: {
        items: [folderRow('smart-folders')],
        selectedThreadIds: [],
        senderReviewSetupComplete: true,
        bulkBusy: false,
        topics: [],
      },
    });
    expect(withoutTopics.findComponent(TopicsFolderCard).exists()).toBe(false);

    const withTopics = shallowMount(ThreadGroupList, {
      props: {
        items: [folderRow('smart-folders')],
        selectedThreadIds: [],
        senderReviewSetupComplete: true,
        bulkBusy: false,
        topics: [{ id: 'topic-1' } as never],
      },
    });
    expect(withTopics.findComponent(TopicsFolderCard).exists()).toBe(true);
  });
});
