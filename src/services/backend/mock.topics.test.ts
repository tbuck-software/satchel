// @vitest-environment jsdom
import { expect, it } from 'vitest';
import { sendCommand } from './mock';

it('topic membership receipts distinguish duplicate adds and absent removals in the mock transport', async () => {
  const topics = await sendCommand<{ id: string }[]>('list_topics');
  const topicId = topics[0]!.id;
  const input = { topic_id: topicId, item_type: 'thread', item_ids: ['one', 'one', 'two'] };
  expect(await sendCommand('add_topic_items', { input })).toEqual(['one', 'two']);
  expect(await sendCommand('add_topic_items', { input })).toEqual([]);
  expect(await sendCommand('list_topic_memberships', { input: { item_type: 'thread', item_ids: ['one'] } }))
    .toEqual([{ topic_id: topicId, item_id: 'one' }]);
  const remove = { topic_id: topicId, item_type: 'thread', item_id: 'one' };
  expect(await sendCommand('remove_topic_item', { input: remove })).toBe(true);
  expect(await sendCommand('remove_topic_item', { input: remove })).toBe(false);
});
