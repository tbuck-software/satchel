<script setup lang="ts">
import IconButton from '@/components/ui/IconButton.vue';

withDefaults(defineProps<{ bundleCount?: number; busy?: boolean }>(), { bundleCount: 0, busy: false });
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
      :title="bundleCount ? $t('thread.bundle.checkAllTitle', { count: bundleCount }) : $t('common.done')"
      :aria-label="bundleCount ? $t('thread.bundle.checkAllAria', { count: bundleCount }) : $t('common.done')"
      size="sm"
      class="thread-quick-action thread-quick-action--done"
      :disabled="busy"
      @click="emit('action', 'done', $event)"
    >
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <template v-if="bundleCount">
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
