// @vitest-environment jsdom

import { defineComponent } from 'vue';
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  route: { params: { id: 'topic-1' } },
  push: vi.fn(),
}));

vi.mock('vue-router', () => ({
  useRoute: () => harness.route,
  useRouter: () => ({ push: harness.push }),
}));

import TopicView from './TopicView.vue';

const TopicDetailStub = defineComponent({
  name: 'TopicDetail',
  props: { topicId: { type: String, required: true } },
  emits: ['deleted', 'open-thread'],
  template: `
    <div data-test="topic-detail" :data-topic-id="topicId">
      <button data-test="open-thread" @click="$emit('open-thread', 'thread-42')">Open thread</button>
      <button data-test="delete-topic" @click="$emit('deleted')">Delete topic</button>
    </div>
  `,
});

function render() {
  return mount(TopicView, {
    global: { stubs: { TopicDetail: TopicDetailStub } },
  });
}

describe('TopicView', () => {
  beforeEach(() => {
    harness.route.params.id = 'topic-1';
    harness.push.mockReset();
  });

  it('passes the routed topic id to the detail view', () => {
    const wrapper = render();

    expect(wrapper.get('[data-test="topic-detail"]').attributes('data-topic-id')).toBe('topic-1');
  });

  it('opens the selected thread in the thread detail route', async () => {
    const wrapper = render();

    await wrapper.get('[data-test="open-thread"]').trigger('click');

    expect(harness.push).toHaveBeenCalledWith({ name: 'thread', params: { id: 'thread-42' } });
  });

  it('returns to the inbox after the topic is deleted', async () => {
    const wrapper = render();

    await wrapper.get('[data-test="delete-topic"]').trigger('click');

    expect(harness.push).toHaveBeenCalledWith({ name: 'inbox' });
  });
});
