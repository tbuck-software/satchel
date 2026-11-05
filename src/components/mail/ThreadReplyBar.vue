<script setup lang="ts">
import { ref } from 'vue';
import CardButton from '@/components/ui/CardButton.vue';
import AppButton from '@/components/ui/AppButton.vue';
import { useToastClearance } from '@/composables/useToastClearance';

/*
 * The foot of the reader: who you would be answering, as whom, and one way
 * in. Not a composer — drafts, signatures, attachments, scheduling and send
 * live in ComposerHost, and a second text field here would either duplicate
 * that or quietly lose what you typed into it on the way over. The bar shows
 * the two things you would otherwise open the composer to find out. While
 * the thread has an unsent draft the bar is not shown: the draft card is the
 * reply in progress.
 */
defineProps<{
  recipient: string;
  identity: string;
  identityColor?: string | null;
}>();

defineEmits<{
  reply: [];
}>();

// The bar ends the reader in the corner the toast stack starts in.
const barRef = ref<HTMLElement | null>(null);
useToastClearance(barRef);
</script>

<template>
  <div ref="barRef" class="reply-bar">
    <CardButton class="reply-bar__field" @click="$emit('reply')">
      <span class="reply-bar__placeholder">
        {{ $t('thread.reply.to', { name: recipient }) }}
      </span>
      <!-- Which account answers, as a quiet chip against the button it
           qualifies. The dot is the account's colour, the marker the
           sidebar and the list draw for it. -->
      <span v-if="identity" class="reply-bar__identity">
        <span
          v-if="identityColor"
          class="reply-bar__dot"
          :style="{ '--reply-identity-color': identityColor }"
          aria-hidden="true"
        />
        <span class="reply-bar__identity-text">{{ $t('thread.reply.as', { identity }) }}</span>
      </span>
    </CardButton>
    <AppButton variant="primary" size="sm" @click="$emit('reply')">
      <template #icon>
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M6 3L1 8l5 5" />
          <path d="M1 8h9a5 5 0 0 1 5 5" />
        </svg>
      </template>
      {{ $t('common.reply') }}
    </AppButton>
  </div>
</template>

<style scoped>
.reply-bar {
  display: flex;
  align-items: center;
  gap: var(--space-md);
  padding: var(--space-md) var(--space-lg);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  box-shadow: var(--shadow-sm);
}

.reply-bar__field {
  flex: 1 1 auto;
  align-items: center;
  gap: var(--space-md);
}

.reply-bar__placeholder {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-tertiary);
  font-size: var(--font-size-md);
}

/* At the end of the field, against the Reply button, rather than wherever
   the placeholder happens to end. The placeholder gives way first; a long
   account name stops at half the field and ends in an ellipsis. */
.reply-bar__identity {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1-5);
  flex: 0 0 auto;
  min-width: 0;
  max-width: 50%;
  height: 24px;
  padding: 0 var(--space-sm) 0 var(--space-1-5);
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  /* Which account answers is a fact to check, not a placeholder. */
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-medium);
}

.reply-bar__identity-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* The account's colour as a swatch, not a signal: a touch larger than the
   old 7px dot and edged in the chip's fill, so a saturated red reads as a
   marker beside the name rather than an alert in the middle of the bar. */
.reply-bar__dot {
  flex: 0 0 auto;
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--reply-identity-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--reply-identity-color) 22%, var(--bg-raised));
}
</style>
