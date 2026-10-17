// @vitest-environment jsdom

import { defineComponent } from 'vue';
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import type { Message } from '@/types/mail';
import ThreadMessageList from './ThreadMessageList.vue';

const MessageBubbleStub = defineComponent({
  name: 'MessageBubble',
  props: {
    message: { type: Object, required: true },
    canReply: Boolean,
    draftBusy: Boolean,
    collapsed: Boolean,
    keyboardFocused: Boolean,
  },
  emits: ['reply', 'forward', 'edit-draft', 'discard-draft', 'focus-message', 'toggle-collapsed'],
  template: '<article :data-message-id="message.id" />',
});

function message(id: string, isDraft = false): Message {
  return {
    id,
    threadId: 'thread-1',
    from: { name: 'Lena', email: 'lena@example.com' },
    to: [],
    subject: 'Thread',
    bodyText: id,
    timestamp: '2026-08-31T00:00:00Z',
    isRead: true,
    isDraft,
    attachments: [],
  };
}

function render() {
  return mount(ThreadMessageList, {
    props: {
      messages: [message('message-1')],
      draftMessages: [message('draft-1', true)],
      canReply: true,
      draftBusy: (messageId: string) => messageId === 'draft-1',
      collapsedMessageIds: new Set(['message-1']),
      focusedMessageId: 'draft-1',
      keyboardActive: true,
    },
    global: {
      mocks: {
        $t: (key: string, params?: { count?: number }) =>
          params?.count === undefined ? key : `${key}:${params.count}`,
      },
      stubs: { MessageBubble: MessageBubbleStub },
    },
  });
}

describe('ThreadMessageList', () => {
  it('renders regular messages before a labelled draft group and forwards display state', () => {
    const wrapper = render();
    const bubbles = wrapper.findAllComponents(MessageBubbleStub);

    expect(bubbles.map((bubble) => bubble.props('message').id)).toEqual(['message-1', 'draft-1']);
    expect(bubbles[0]!.props()).toMatchObject({
      canReply: true,
      draftBusy: false,
      collapsed: true,
      keyboardFocused: false,
    });
    expect(bubbles[1]!.props()).toMatchObject({
      canReply: true,
      draftBusy: true,
      collapsed: false,
      keyboardFocused: true,
    });
    expect(wrapper.get('section').attributes('aria-label')).toBe('thread.view.unsentDrafts');
    expect(wrapper.get('section').text()).toContain('thread.view.draftDivider:1');
  });

  it('forwards message actions with the owning message id', () => {
    const wrapper = render();
    const [messageBubble, draftBubble] = wrapper.findAllComponents(MessageBubbleStub);

    messageBubble!.vm.$emit('reply');
    messageBubble!.vm.$emit('forward');
    messageBubble!.vm.$emit('focus-message');
    messageBubble!.vm.$emit('toggle-collapsed');
    draftBubble!.vm.$emit('edit-draft', 'draft-1');
    draftBubble!.vm.$emit('discard-draft', 'draft-1');

    expect(wrapper.emitted('reply')).toHaveLength(1);
    expect(wrapper.emitted('forward')).toHaveLength(1);
    expect(wrapper.emitted('focusMessage')).toEqual([['message-1']]);
    expect(wrapper.emitted('toggleCollapsed')).toEqual([['message-1']]);
    expect(wrapper.emitted('editDraft')).toEqual([['draft-1']]);
    expect(wrapper.emitted('discardDraft')).toEqual([['draft-1']]);
  });

  it('omits the draft divider when there are no drafts', async () => {
    const wrapper = render();
    await wrapper.setProps({ draftMessages: [] });

    expect(wrapper.find('section').exists()).toBe(false);
  });
});
