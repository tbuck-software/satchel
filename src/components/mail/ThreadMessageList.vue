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

  <section
    v-if="props.draftMessages.length > 0"
    class="mt-6"
    :aria-label="$t('thread.view.unsentDrafts')"
  >
    <div class="mb-3 flex items-center gap-3" aria-hidden="true">
      <span class="h-px flex-1 bg-border/70" />
      <span class="text-xs font-semibold uppercase tracking-wide text-tertiary">
        {{ $t('thread.view.draftDivider', { count: props.draftMessages.length }) }}
      </span>
      <span class="h-px flex-1 bg-border/70" />
    </div>
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
