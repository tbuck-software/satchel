// @vitest-environment jsdom
import { shallowMount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import type { ThreadListEntry } from '@/utils/threadList';
import ThreadGroupList from './ThreadGroupList.vue';

/*
 * A listbox may own options and nothing else. The smart view puts headers,
 * cards and buttons between the rows, so the rows are split into one listbox
 * per run and everything else sits beside them.
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
    priority: 1 as const,
    mailboxNames: ['INBOX'],
    tags: [],
  };
}

function bucket(id: string, label: string) {
  return {
    id,
    key: 'other',
    label,
    tone: 'other',
    count: 2,
    unreadCount: 0,
    threadIds: ['b1', 'b2'],
    previewText: '',
    previewItems: [],
  };
}

const row = (id: string) => ({ kind: 'thread', id: `row-${id}`, thread: thread(id) });
const header = (id: string, label: string) => ({ kind: 'date-header', id, label, count: 0, threadIds: [] });

const ITEMS = [
  {
    kind: 'priority-card',
    id: 'priority-card',
    bucket: bucket('bucket-priority', 'Priority'),
    threads: [thread('p1'), thread('p2')],
    hiddenCount: 0,
  },
  header('today', 'Today'),
  row('a'),
  row('b'),
  { kind: 'bucket', id: 'bucket-row', bucket: bucket('bucket-news', 'Newsletters') },
  row('c'),
  header('yesterday', 'Yesterday'),
  row('d'),
] as unknown as ThreadListEntry[];

function mountList() {
  return shallowMount(ThreadGroupList, {
    props: {
      items: ITEMS,
      selectedThreadIds: [],
      senderReviewSetupComplete: true,
      bulkBusy: false,
    },
    global: {
      stubs: {
        ThreadListItem: {
          props: ['thread', 'posInSet', 'setSize'],
          template: '<div role="option" :data-thread="thread.id" :aria-posinset="posInSet" :aria-setsize="setSize" />',
        },
      },
    },
  });
}

describe('thread list accessibility tree', () => {
  it('wraps the scroller in a neutral group, not a listbox', () => {
    const root = mountList().get('.thread-list-scroller');

    expect(root.attributes('role')).toBe('group');
    expect(root.attributes('aria-label')).toBe('Message list');
  });

  it('gives each run of rows its own labelled listbox', () => {
    const listboxes = mountList().findAll('[role="listbox"]');

    expect(listboxes.map((box) => [
      box.attributes('aria-label'),
      box.findAll('[role="option"]').map((option) => option.attributes('data-thread')),
    ])).toEqual([
      ['Priority', ['p1', 'p2']],
      ['Today', ['a', 'b']],
      ['Today', ['c']],
      ['Yesterday', ['d']],
    ]);
    for (const box of listboxes) expect(box.attributes('aria-multiselectable')).toBe('true');
  });

  it('owns nothing but options', () => {
    for (const box of mountList().findAll('[role="listbox"]')) {
      const strays = box.findAll('*').filter((node) => {
        const role = node.attributes('role');
        return role !== 'option' && role !== 'presentation' && !node.element.closest('[role="option"]');
      });
      expect(strays.map((node) => node.element.outerHTML)).toEqual([]);
    }
  });

  it('numbers the rows within their own listbox', () => {
    const rows = mountList().findAll('[role="option"]');

    expect(rows.map((option) => [
      option.attributes('data-thread'),
      option.attributes('aria-posinset'),
      option.attributes('aria-setsize'),
    ])).toEqual([
      ['p1', '1', '2'],
      ['p2', '2', '2'],
      ['a', '1', '2'],
      ['b', '2', '2'],
      ['c', '1', '1'],
      ['d', '1', '1'],
    ]);
  });
});
