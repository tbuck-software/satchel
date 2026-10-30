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
    collapsed: Boolean,
    keyboardFocused: Boolean,
  },
  emits: ['reply', 'forward', 'focus-message', 'toggle-collapsed'],
  template: '<article :data-message-id="message.id" />',
});

const DraftCardStub = defineComponent({
  name: 'ThreadDraftCard',
  props: {
    message: { type: Object, required: true },
    recipient: String,
    identity: String,
    identityColor: String,
    busy: Boolean,
    keyboardFocused: Boolean,
  },
  emits: ['edit', 'discard', 'focus', 'retry'],
  template: '<article :data-draft-id="message.id" />',
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
      draftRecipient: (draft: Message) => `to:${draft.id}`,
      draftIdentity: 'Work',
      collapsedMessageIds: new Set(['message-1']),
      focusedMessageId: 'draft-1',
      keyboardActive: true,
    },
    global: {
      mocks: {
        $t: (key: string, params?: { count?: number }) =>
          params?.count === undefined ? key : `${key}:${params.count}`,
      },
      stubs: { MessageBubble: MessageBubbleStub, ThreadDraftCard: DraftCardStub },
    },
  });
}

describe('ThreadMessageList', () => {
  it('renders messages as bubbles and each draft as one card after them', () => {
    const wrapper = render();
    const bubbles = wrapper.findAllComponents(MessageBubbleStub);
    const cards = wrapper.findAllComponents(DraftCardStub);

    // A draft is never a message bubble: one object per draft, the card.
    expect(bubbles.map((bubble) => bubble.props('message').id)).toEqual(['message-1']);
    expect(bubbles[0]!.props()).toMatchObject({ canReply: true, collapsed: true, keyboardFocused: false });
    expect(cards.map((card) => card.props('message').id)).toEqual(['draft-1']);
    expect(cards[0]!.props()).toMatchObject({
      recipient: 'to:draft-1',
      identity: 'Work',
      busy: true,
      keyboardFocused: true,
    });
    expect(wrapper.get('section').attributes('aria-label')).toBe('thread.view.unsentDrafts');
  });

  it('forwards message and draft actions with the owning message id', () => {
    const wrapper = render();
    const messageBubble = wrapper.findComponent(MessageBubbleStub);
    const draftCard = wrapper.findComponent(DraftCardStub);

    messageBubble.vm.$emit('reply');
    messageBubble.vm.$emit('forward');
    messageBubble.vm.$emit('focus-message');
    messageBubble.vm.$emit('toggle-collapsed');
    draftCard.vm.$emit('edit');
    draftCard.vm.$emit('retry');
    draftCard.vm.$emit('discard');
    draftCard.vm.$emit('focus');

    expect(wrapper.emitted('reply')).toHaveLength(1);
    expect(wrapper.emitted('forward')).toHaveLength(1);
    expect(wrapper.emitted('focusMessage')).toEqual([['message-1'], ['draft-1']]);
    expect(wrapper.emitted('toggleCollapsed')).toEqual([['message-1']]);
    expect(wrapper.emitted('editDraft')).toEqual([['draft-1']]);
    expect(wrapper.emitted('retryDraft')).toEqual([['draft-1']]);
    expect(wrapper.emitted('discardDraft')).toEqual([['draft-1']]);
  });

  it('omits the draft group when there are no drafts', async () => {
    const wrapper = render();
    await wrapper.setProps({ draftMessages: [] });

    expect(wrapper.find('section').exists()).toBe(false);
  });
});
