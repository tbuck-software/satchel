// @vitest-environment jsdom
import { expect, it } from 'vitest';
import { sendCommand } from './mock';
import type { TopicMembershipReceipt } from './topics';
import type { BackendTopicTimelineItem } from '@/types/topics';

it('topic membership receipts distinguish duplicate adds, absent removals and ABA Undo', async () => {
  const topics = await sendCommand<{ id: string }[]>('list_topics');
  const topicId = topics[0]!.id;
  const input = { topic_id: topicId, item_type: 'thread', item_ids: ['one', 'one', 'two'] };
  const first = await sendCommand<TopicMembershipReceipt[]>('add_topic_items', { input });
  expect(first.map((row) => row.item_id)).toEqual(['one', 'two']);
  expect(await sendCommand('add_topic_items', { input })).toEqual([]);
  expect(await sendCommand('list_topic_memberships', { input: { item_type: 'thread', item_ids: ['one'] } }))
    .toEqual([{ topic_id: topicId, item_id: 'one' }]);
  const remove = { topic_id: topicId, item_type: 'thread', item_id: 'one' };
  expect(await sendCommand('remove_topic_item', { input: remove })).toMatchObject({ item_id: 'one' });
  expect(await sendCommand('remove_topic_item', { input: remove })).toBeNull();
  await sendCommand('add_topic_items', { input });
  expect(await sendCommand('undo_topic_memberships', { input: { topic_id: topicId, item_type: 'thread', receipts: first, restore: false } })).toEqual(['two']);
});

it('reopening a timeline agrees with memberships and preserves unrelated notes and message metadata', async () => {
  const topics = await sendCommand<{ id: string }[]>('list_topics');
  const topicId = topics[0]!.id;
  const before = await sendCommand<BackendTopicTimelineItem[]>('get_topic_timeline', { topicId });
  const thread = before.find((row) => row.item_type === 'thread')!;
  expect(thread).toBeDefined();
  const input = { topic_id: topicId, item_type: 'thread', item_id: thread.item_id };
  const receipt = await sendCommand<TopicMembershipReceipt>('remove_topic_item', { input });
  const removed = await sendCommand<BackendTopicTimelineItem[]>('get_topic_timeline', { topicId });
  expect(removed).toEqual(before.filter((row) => row !== thread));
  expect(await sendCommand('list_topic_memberships', { input: { item_type: 'thread', item_ids: [thread.item_id] } }))
    .not.toContainEqual({ topic_id: topicId, item_id: thread.item_id });
  await sendCommand('undo_topic_memberships', { input: { topic_id: topicId, item_type: 'thread', receipts: [receipt], restore: true } });
  expect(await sendCommand<BackendTopicTimelineItem[]>('get_topic_timeline', { topicId })).toContainEqual(thread);
  const messages = await sendCommand<{ id: string }[]>('list_messages_preview', { threadId: thread.thread_id });
  const messageId = messages[0]!.id;
  await sendCommand('add_topic_items', { input: { topic_id: topicId, item_type: 'message', item_ids: [messageId], added_by: null } });
  const added = await sendCommand<BackendTopicTimelineItem[]>('get_topic_timeline', { topicId });
  expect(added).toContainEqual(expect.objectContaining({ item_type: 'message', item_id: messageId, thread_id: thread.thread_id, title: expect.any(String), added_by: 'user' }));
  expect(added.filter((row) => row.item_type === 'note')).toEqual(before.filter((row) => row.item_type === 'note'));
});


it('deleting a topic retires removed-membership receipts as well as present links', async () => {
  const topics = await sendCommand<{ id: string }[]>('list_topics');
  const topicId = topics.at(-1)!.id;
  const input = { topic_id: topicId, item_type: 'event', item_id: 'deleted-event' };
  await sendCommand('add_topic_items', { input: { topic_id: topicId, item_type: 'event', item_ids: [input.item_id] } });
  const receipt = await sendCommand<TopicMembershipReceipt>('remove_topic_item', { input });
  await sendCommand('delete_topic', { id: topicId });
  expect(await sendCommand('undo_topic_memberships', { input: { topic_id: topicId, item_type: 'event', receipts: [receipt], restore: true } })).toEqual([]);
  expect(await sendCommand('get_topic_timeline', { topicId })).toEqual([]);
});
