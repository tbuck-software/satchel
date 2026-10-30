// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import TopicMembershipState from './TopicMembershipState.vue';
import { useTopicsStore } from '@/stores/topics';

const list = vi.hoisted(() => vi.fn());
vi.mock('@/services/backend', async (original) => ({
  ...await original<typeof import('@/services/backend')>(),
  backendCommandsAvailable: () => true,
  backendListTopicMemberships: list,
}));
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => ({ visibilityKey: 'a' }) }));
vi.mock('@/stores/connection', () => ({ useConnectionStore: () => ({
  reportBackendError: vi.fn(), clearBackendError: vi.fn(), retryToken: 0,
}) }));
function deferred() {
  let resolve!: (rows: Array<{ item_id: string; topic_id: string }>) => void;
  const promise = new Promise<Array<{ item_id: string; topic_id: string }>>((done) => { resolve = done; });
  return { resolve, promise };
}
beforeEach(() => {
  setActivePinia(createPinia());
  vi.useFakeTimers();
  list.mockReset().mockResolvedValue([]);
});
afterEach(() => vi.useRealTimers());

it('shows delayed progress while refreshing retained memberships', async () => {
  const topics = useTopicsStore();
  await topics.loadMembershipsFor('thread', ['one']);
  const wrapper = mount(TopicMembershipState, { props: { itemType: 'thread', itemIds: ['one'] } });
  expect(wrapper.attributes('aria-busy')).toBe('false');
  const gate = deferred();
  list.mockReturnValueOnce(gate.promise);
  const pending = topics.loadMembershipsFor('thread', ['one']);
  await nextTick();
  expect(wrapper.attributes('aria-busy')).toBe('true');
  await vi.advanceTimersByTimeAsync(159);
  expect(wrapper.find('[role="status"]').exists()).toBe(false);
  await vi.advanceTimersByTimeAsync(1);
  expect(wrapper.find('.collection-refresh[role="status"]').exists()).toBe(true);
  gate.resolve([]);
  await pending;
  await nextTick();
  expect(wrapper.attributes('aria-busy')).toBe('false');
  expect(wrapper.find('[role="status"]').exists()).toBe(false);
  wrapper.unmount();
});

it('shows loading during Retry and keeps newer requests busy when an older one completes', async () => {
  const topics = useTopicsStore();
  list.mockRejectedValueOnce(new Error('offline'));
  await topics.loadMembershipsFor('thread', ['one']);
  const wrapper = mount(TopicMembershipState, { props: { itemType: 'thread', itemIds: ['one'] } });
  expect(wrapper.find('[role="alert"]').exists()).toBe(true);
  const first = deferred();
  const second = deferred();
  list.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  await wrapper.get('button').trigger('click');
  const newer = topics.loadMembershipsFor('thread', ['one']);
  await nextTick();
  expect(wrapper.attributes('aria-busy')).toBe('true');
  await vi.advanceTimersByTimeAsync(160);
  expect(wrapper.find('[role="status"]').exists()).toBe(true);
  expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  first.resolve([]);
  await vi.advanceTimersByTimeAsync(1);
  expect(wrapper.attributes('aria-busy')).toBe('true');
  second.resolve([{ item_id: 'one', topic_id: 'topic' }]);
  await newer;
  await nextTick();
  expect(wrapper.attributes('aria-busy')).toBe('false');
  expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  expect(topics.topicIdsFor('thread', 'one')).toEqual(['topic']);
  wrapper.unmount();
});
