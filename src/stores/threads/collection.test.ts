import { expect, it } from 'vitest';
import { isReactive } from 'vue';
import { createThreadCollection } from './collection';
import type { Thread } from '@/types/mail';

const row = (id: string): Thread => ({ id, accountId: 'acc', subject: id, snippet: '',
  participants: [{ name: 'Sender', email: 'a@example.com' }], sentParticipants: [],
  lastMessageAt: '2026-08-01', lastMessageAtMs: 0, messageCount: 1, unreadCount: 2,
  hasAttachments: false, workflowState: 'inbox', priority: 1, mailboxNames: ['INBOX'], tags: [] });

it('keeps raw rows and the array when a snapshot is unchanged', () => {
  const collection = createThreadCollection([row('a'), row('b')]);
  const before = collection.threads.value;
  expect(isReactive(before[0])).toBe(false);
  collection.replaceAll(structuredClone(before));
  expect(collection.threads.value).toBe(before);
  collection.appendUnique([]);
  collection.remove('missing');
  expect(collection.threads.value).toBe(before);
});

it('reuses only unchanged rows, including nested values and optional fields', () => {
  const collection = createThreadCollection([row('a'), row('b')]);
  const before = collection.threads.value;
  collection.replaceAll([structuredClone(before[0]!), { ...row('b'), participants: [{ name: 'New', email: 'b@example.com' }], isStarred: true }]);
  expect(collection.find('a')).toBe(before[0]);
  expect(collection.find('b')).not.toBe(before[1]);
  expect(collection.find('b')?.isStarred).toBe(true);
});

it('publishes immutable workflow and read mutations without changing other rows', () => {
  const collection = createThreadCollection([row('a'), row('b')]);
  const before = collection.threads.value;
  collection.applyWorkflowStates([{ threadId: 'a', state: 'done' }]);
  expect(before[0]?.workflowState).toBe('inbox');
  expect(collection.find('a')?.workflowState).toBe('done');
  expect(collection.find('b')).toBe(before[1]);
  collection.markRead('a', true);
  expect(collection.find('a')?.unreadCount).toBe(0);
  const read = collection.threads.value;
  collection.markRead('a', true);
  expect(collection.threads.value).toBe(read);
});

it('patches a row without rebuilding the id index', () => {
  const collection = createThreadCollection([row('a'), row('b'), row('c')]);
  collection.find('a');
  const builds = collection.indexBuilds();
  const before = collection.threads.value;

  collection.markRead('b', true);

  expect(collection.threads.value).not.toBe(before);
  expect(collection.find('b')?.unreadCount).toBe(0);
  expect(collection.find('c')).toBe(before[2]);
  expect(collection.indexBuilds()).toBe(builds);

  // A patch that changes nothing publishes nothing.
  const read = collection.threads.value;
  collection.markRead('b', true);
  collection.applyWorkflowStates([{ threadId: 'missing', state: 'done' }]);
  expect(collection.threads.value).toBe(read);
});
