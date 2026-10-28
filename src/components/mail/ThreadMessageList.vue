<script setup lang="ts">
import { computed } from 'vue';
import type { Message } from '@/types/mail';
import ThreadMessageGroup from './ThreadMessageGroup.vue';
import ThreadDraftCard from './ThreadDraftCard.vue';

const props = defineProps<{
  messages: Message[];
  draftMessages: Message[];
  canReply: boolean;
  draftBusy: (messageId: string) => boolean;
  /** Who a draft is addressed to, as the thread knows them. */
  draftRecipient: (message: Message) => string;
  /** The identity a draft would go out as. */
  draftIdentity?: string;
  draftIdentityColor?: string | null;
  collapsedMessageIds: ReadonlySet<string>;
  /** Messages whose renderer may mount; without it, every open message. */
  bodyMountedIds?: ReadonlySet<string>;
  focusedMessageId: string;
  keyboardActive: boolean;
  /** The draft the composer has open, if it is one of these. */
  editingDraftId?: string | null;
}>();

const emit = defineEmits<{
  reply: [];
  forward: [];
  editDraft: [messageId: string];
  discardDraft: [messageId: string];
  focusMessage: [messageId: string];
  toggleCollapsed: [messageId: string];
}>();

// The composer holding a draft is the draft; a second copy beside it would
// fall behind the typing.
const shownDrafts = computed(() => props.draftMessages.filter((draft) => draft.id !== props.editingDraftId));
</script>

<template>
  <ThreadMessageGroup
    :messages="props.messages"
    :can-reply="props.canReply"
    :collapsed-message-ids="props.collapsedMessageIds"
    :body-mounted-ids="props.bodyMountedIds"
    :focused-message-id="props.focusedMessageId"
    :keyboard-active="props.keyboardActive"
    @reply="emit('reply')"
    @forward="emit('forward')"
    @focus-message="emit('focusMessage', $event)"
    @toggle-collapsed="emit('toggleCollapsed', $event)"
  />

  <!-- The drafts end the thread, each one card that carries its own way back
       in, so the group needs no divider over it. -->
  <section
    v-if="shownDrafts.length > 0"
    :aria-label="$t('thread.view.unsentDrafts')"
  >
    <ThreadDraftCard
      v-for="draft in shownDrafts"
      :key="draft.id"
      :message="draft"
      :recipient="props.draftRecipient(draft)"
      :identity="props.draftIdentity"
      :identity-color="props.draftIdentityColor"
      :busy="props.draftBusy(draft.id)"
      :keyboard-focused="props.keyboardActive && props.focusedMessageId === draft.id"
      @edit="emit('editDraft', draft.id)"
      @discard="emit('discardDraft', draft.id)"
      @focus="emit('focusMessage', draft.id)"
    />
  </section>
</template>
