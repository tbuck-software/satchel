<script setup lang="ts">
import { Check, CheckCheck, Clock, Inbox } from 'lucide';
import AppIcon from '@/components/ui/AppIcon.vue';
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
      <AppIcon :icon="Clock" />
    </IconButton>

    <IconButton
      :title="done ? $t('thread.actions.moveToInbox') : bundleCount ? $t('thread.bundle.checkAllTitle', { count: bundleCount }) : $t('common.done')"
      :aria-label="done ? $t('thread.actions.moveToInbox') : bundleCount ? $t('thread.bundle.checkAllAria', { count: bundleCount }) : $t('common.done')"
      size="sm"
      class="thread-quick-action thread-quick-action--done"
      :disabled="busy"
      @click="emit('action', 'done', $event)"
    >
      <AppIcon :icon="done ? Inbox : bundleCount ? CheckCheck : Check" />
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
