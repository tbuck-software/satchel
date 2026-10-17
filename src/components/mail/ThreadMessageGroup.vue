<script setup lang="ts">
import type { Message } from '@/types/mail';
import MessageBubble from './MessageBubble.vue';

const props = defineProps<{
  messages: Message[];
  canReply: boolean;
  draftBusy: (messageId: string) => boolean;
  collapsedMessageIds: ReadonlySet<string>;
  /** Messages whose renderer may mount; without it, every open message. */
  bodyMountedIds?: ReadonlySet<string>;
  focusedMessageId: string;
  keyboardActive: boolean;
}>();

const emit = defineEmits<{
  reply: [];
  forward: [];
  editDraft: [messageId: string];
  discardDraft: [messageId: string];
  focusMessage: [messageId: string];
  toggleCollapsed: [messageId: string];
}>();
</script>

<template>
  <MessageBubble
    v-for="message in props.messages"
    :key="message.id"
    :message="message"
    :can-reply="props.canReply"
    :draft-busy="props.draftBusy(message.id)"
    :collapsed="props.collapsedMessageIds.has(message.id)"
    :body-mounted="props.bodyMountedIds?.has(message.id) ?? true"
    :keyboard-focused="props.keyboardActive && props.focusedMessageId === message.id"
    @reply="emit('reply')"
    @forward="emit('forward')"
    @edit-draft="emit('editDraft', $event)"
    @discard-draft="emit('discardDraft', $event)"
    @focus-message="emit('focusMessage', message.id)"
    @toggle-collapsed="emit('toggleCollapsed', message.id)"
  />
</template>
