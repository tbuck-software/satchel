<script setup lang="ts">
import type { Message } from '@/types/mail';
import MessageBubble from './MessageBubble.vue';

const props = defineProps<{
  messages: Message[];
  canReply: boolean;
  collapsedMessageIds: ReadonlySet<string>;
  /** Messages whose renderer may mount; without it, every open message. */
  bodyMountedIds?: ReadonlySet<string>;
  focusedMessageId: string;
  keyboardActive: boolean;
}>();

const emit = defineEmits<{
  reply: [];
  forward: [];
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
    :collapsed="props.collapsedMessageIds.has(message.id)"
    :body-mounted="props.bodyMountedIds?.has(message.id) ?? true"
    :keyboard-focused="props.keyboardActive && props.focusedMessageId === message.id"
    @reply="emit('reply')"
    @forward="emit('forward')"
    @focus-message="emit('focusMessage', message.id)"
    @toggle-collapsed="emit('toggleCollapsed', message.id)"
  />
</template>
