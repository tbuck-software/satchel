<script setup lang="ts">
import IconButton from '@/components/ui/IconButton.vue';

/*
 * `done`: the mail is done already, so the check would do nothing; the
 * button sends it back to the Inbox instead. The event stays `done` and the
 * parent's toggle decides.
 */
withDefaults(defineProps<{ bundleCount?: number; busy?: boolean; done?: boolean }>(), { bundleCount: 0, busy: false, done: false });
type QuickAction = 'snooze' | 'done';

const emit = defineEmits<{
  action: [action: QuickAction, event: MouseEvent];
}>();
</script>

<template>
  <div
    class="flex items-center justify-end gap-1"
  >
    <IconButton
      v-if="!bundleCount"
      :title="$t('thread.actions.snooze')"
      :aria-label="$t('thread.actions.snooze')"
      size="sm"
      class="thread-quick-action"
      :disabled="busy"
      @click="emit('action', 'snooze', $event)"
    >
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="8" cy="8" r="6" />
        <path d="M8 4v4l2.5 1.5" />
      </svg>
    </IconButton>

    <IconButton
      :title="done ? $t('thread.actions.moveToInbox') : bundleCount ? $t('thread.bundle.checkAllTitle', { count: bundleCount }) : $t('common.done')"
      :aria-label="done ? $t('thread.actions.moveToInbox') : bundleCount ? $t('thread.bundle.checkAllAria', { count: bundleCount }) : $t('common.done')"
      size="sm"
      class="thread-quick-action thread-quick-action--done"
      :disabled="busy"
      @click="emit('action', 'done', $event)"
    >
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path v-if="done" d="M8 11V3.4M5.4 8.4 8 11l2.6-2.6M2.8 11.2v1.2a1 1 0 0 0 1 1h8.4a1 1 0 0 0 1-1v-1.2" />
        <template v-else-if="bundleCount">
          <path d="M1.4 8.2l2.4 2.4 4.5-5" />
          <path d="M6.4 10.7l2.1 2.1 6-6.6" />
        </template>
        <path v-else d="M4 8l3 3 5-6" />
      </svg>
    </IconButton>
  </div>
</template>

<style scoped>
.thread-quick-action {
  background: var(--bg-raised);
}

.thread-quick-action.thread-quick-action--done:is(:hover, :focus-visible):not(:disabled) {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}
</style>
