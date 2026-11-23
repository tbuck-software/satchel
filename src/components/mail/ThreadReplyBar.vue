<script setup lang="ts">
import { computed, ref } from 'vue';
import { Reply } from 'lucide';
import AccountAvatar from '@/components/common/AccountAvatar.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CardButton from '@/components/ui/CardButton.vue';
import KeyCap from '@/components/ui/KeyCap.vue';
import { useShortcutHints } from '@/composables/useShortcutHints';
import { useToastClearance } from '@/composables/useToastClearance';

/*
 * The foot of the reader: who you would be answering, as whom, and one way
 * in. Not a composer — drafts, signatures, attachments, scheduling and send
 * live in ComposerHost, and a second text field here would either duplicate
 * that or quietly lose what you typed into it on the way over. The bar shows
 * the two things you would otherwise open the composer to find out. While
 * the thread has an unsent draft the bar is not shown: the draft card is the
 * reply in progress.
 *
 * One field, one button. It used to be a card holding a field-shaped button
 * and a petrol Reply pill beside it: two targets for one action, a second
 * accent next to the toolbar's Done, and a frame around a frame. The field
 * keeps the message layout's two axes — the reply arrow stands where the
 * avatars do, the prompt starts where names and bodies do — and ends in the
 * key that does the same thing.
 */
defineProps<{
  recipient: string;
  identity: string;
  identityColor?: string | null;
  identityAvatar?: string | null;
}>();

defineEmits<{
  reply: [];
}>();

const { hint } = useShortcutHints();
const replyHint = computed(() => hint('reply'));

// The bar ends the reader in the corner the toast stack starts in.
const barRef = ref<HTMLElement | null>(null);
useToastClearance(barRef);
</script>

<template>
  <div ref="barRef" class="reply-bar">
    <CardButton
      class="reply-bar__field"
      :aria-keyshortcuts="replyHint?.aria"
      @click="$emit('reply')"
    >
      <span class="reply-bar__lead" aria-hidden="true">
        <AppIcon :icon="Reply" />
      </span>
      <span class="reply-bar__prompt">
        {{ $t('thread.reply.to', { name: recipient }) }}
      </span>
      <!-- Which account answers: a fact to check before you start, so it
           reads as text, not as the placeholder. The account's face is the
           one the sidebar lists it under. -->
      <span v-if="identity" class="reply-bar__identity">
        <AccountAvatar :name="identity" :color="identityColor" :avatar="identityAvatar" :size="18" />
        <span class="reply-bar__identity-text">{{ $t('thread.reply.as', { identity }) }}</span>
      </span>
      <span v-if="replyHint" class="reply-bar__keys" aria-hidden="true">
        <KeyCap v-for="key in replyHint.keys" :key="key">{{ key }}</KeyCap>
      </span>
    </CardButton>
  </div>
</template>

<style scoped>
.reply-bar {
  /* The message layout's axes, see MessageBubble --message-text-inset. */
  --reply-lead: 40px;
  --reply-lead-gap: var(--space-3-5);
}

.reply-bar__field {
  align-items: center;
  gap: var(--space-md);
  width: 100%;
  height: 52px;
  /* The border is inside the box, the lead slot starts on the avatar axis. */
  padding: 0 var(--space-md) 0 0;
  border: var(--border-default);
  border-radius: var(--radius-lg);
  background: var(--bg-raised);
  box-shadow: var(--shadow-sm);
  transition:
    background var(--duration-fast) var(--ease-out),
    border-color var(--duration-fast) var(--ease-out);
}

.reply-bar__field:hover {
  border-color: color-mix(in srgb, var(--accent-primary) 45%, var(--border-color));
  background: var(--bg-hover);
}

.reply-bar__field:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

/* As wide as an avatar, less the border, so the arrow centres under the
   avatars above and the prompt begins where their names do. */
.reply-bar__lead {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  justify-content: center;
  width: calc(var(--reply-lead) - 1px);
  margin-inline-end: calc(var(--reply-lead-gap) - var(--space-md));
  color: var(--text-tertiary);
  transition: color var(--duration-fast) var(--ease-out);
}

.reply-bar__field:hover .reply-bar__lead {
  color: var(--text-accent);
}

.reply-bar__prompt {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-tertiary);
  font-size: var(--font-size-md);
}

.reply-bar__field:hover .reply-bar__prompt {
  color: var(--text-secondary);
}

/* The prompt gives way first; a long account name stops at half the field
   and ends in an ellipsis. */
.reply-bar__identity {
  display: inline-flex;
  align-items: center;
  gap: var(--space-sm);
  flex: 0 1 auto;
  min-width: 0;
  max-width: 50%;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.reply-bar__identity-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* A hairline parts the account from the key, so the two facts don't run
   into one. */
.reply-bar__keys {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: var(--space-2xs);
  padding-inline-start: var(--space-md);
  border-inline-start: 1px solid var(--border-color-muted);
}
</style>
