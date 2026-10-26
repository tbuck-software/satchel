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
 * the two things you would otherwise open the composer to find out.
 */
defineProps<{
  recipient: string;
  identity: string;
  identityColor?: string | null;
  /**
   * An unsent draft ends the thread. The way in is then that draft, not a
   * second reply beside it.
   */
  resumesDraft?: boolean;
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
        {{ resumesDraft ? $t('thread.reply.continue', { name: recipient }) : $t('thread.reply.to', { name: recipient }) }}
      </span>
      <span class="reply-bar__identity">
        <span
          v-if="identityColor"
          class="reply-bar__dot"
          :style="{ background: identityColor }"
          aria-hidden="true"
        />
        {{ $t('thread.reply.as', { identity }) }}
      </span>
    </CardButton>
    <AppButton variant="primary" size="sm" @click="$emit('reply')">
      <template #icon>
        <svg v-if="resumesDraft" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M11.2 2.6l2.2 2.2L5.8 12.4 3 13l.6-2.8z" />
          <path d="M2.5 14.5h11" />
        </svg>
        <svg v-else width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M6 3L1 8l5 5" />
          <path d="M1 8h9a5 5 0 0 1 5 5" />
        </svg>
      </template>
      {{ resumesDraft ? $t('thread.reply.resume') : $t('common.reply') }}
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
  align-items: baseline;
  gap: var(--space-md);
}

.reply-bar__placeholder {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-tertiary);
  font-size: var(--font-size-md);
}

.reply-bar__identity {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1-5);
  flex-shrink: 0;
  /* Which account answers is a fact to check, not a placeholder. */
  color: var(--text-secondary);
  font-size: var(--font-size-2xs);
}

.reply-bar__dot {
  width: 7px;
  height: 7px;
  border-radius: var(--radius-full);
}
</style>
