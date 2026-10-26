<script setup lang="ts">
import type { Message } from '@/types/mail';
import ThreadMessageGroup from './ThreadMessageGroup.vue';

const props = defineProps<{
  messages: Message[];
  draftMessages: Message[];
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
  <ThreadMessageGroup
    :messages="props.messages"
    :can-reply="props.canReply"
    :draft-busy="props.draftBusy"
    :collapsed-message-ids="props.collapsedMessageIds"
    :body-mounted-ids="props.bodyMountedIds"
    :focused-message-id="props.focusedMessageId"
    :keyboard-active="props.keyboardActive"
    @reply="emit('reply')"
    @forward="emit('forward')"
    @edit-draft="emit('editDraft', $event)"
    @discard-draft="emit('discardDraft', $event)"
    @focus-message="emit('focusMessage', $event)"
    @toggle-collapsed="emit('toggleCollapsed', $event)"
  />

  <!-- The drafts end the thread as cards of their own; each one says it is
       unsent, so the group needs no divider over it. -->
  <section
    v-if="props.draftMessages.length > 0"
    class="thread-drafts"
    :aria-label="$t('thread.view.unsentDrafts')"
  >
    <ThreadMessageGroup
      :messages="props.draftMessages"
      :can-reply="props.canReply"
      :draft-busy="props.draftBusy"
      :collapsed-message-ids="props.collapsedMessageIds"
      :body-mounted-ids="props.bodyMountedIds"
      :focused-message-id="props.focusedMessageId"
      :keyboard-active="props.keyboardActive"
      @reply="emit('reply')"
      @forward="emit('forward')"
      @edit-draft="emit('editDraft', $event)"
      @discard-draft="emit('discardDraft', $event)"
      @focus-message="emit('focusMessage', $event)"
      @toggle-collapsed="emit('toggleCollapsed', $event)"
    />
  </section>
</template>

<style scoped>
.thread-drafts {
  margin-top: var(--space-md);
}
</style>
