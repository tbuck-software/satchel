<script setup lang="ts">
import { Check, Clock, Ellipsis, Inbox, Mail, MailOpen, Reply } from 'lucide';
import AppIcon from '@/components/ui/AppIcon.vue';
import AppButton from '@/components/ui/AppButton.vue';
import ToolbarCell from '@/components/ui/ToolbarCell.vue';

const props = defineProps<{
  /** Keep the mobile actions mounted while the viewport changes. */
  visible: boolean;
  canReply: boolean;
  isUnread: boolean;
  isDone: boolean;
  selectionCount?: number;
  /** Whether the thread's menu (opened by More) is showing. */
  menuOpen?: boolean;
}>();

const emit = defineEmits<{
  archive: [];
  moveToInbox: [];
  reply: [];
  toggleRead: [];
  snooze: [];
  more: [];
}>();
</script>

<template>
  <!-- ═══ MOBILE BOTTOM BAR ═══
       Four labelled cells, icon over a short label and the whole cell the
       target, beside the prominent Done pill. More opens the thread's one
       menu (ThreadMenuSheet.vue): the conversation, its newest message and
       that message's image choices. It sits here, under the thumb, rather
       than as a ⋯ in the nav bar's far corner. -->
  <div v-if="props.visible" class="mobile-bar">
    <div class="mobile-bar-actions">
      <ToolbarCell
        class="bar-btn"
        :label="$t('common.more')"
        :title="$t('thread.toolbar.moreActions')"
        aria-haspopup="dialog"
        :aria-expanded="props.menuOpen ?? false"
        @click="emit('more')"
      >
        <template #icon>
          <AppIcon :icon="Ellipsis" size="lg" />
        </template>
      </ToolbarCell>
      <ToolbarCell
        v-if="props.canReply"
        class="bar-btn"
        :label="$t('common.reply')"
        @click="emit('reply')"
      >
        <template #icon>
          <AppIcon :icon="Reply" size="lg" />
        </template>
      </ToolbarCell>
      <ToolbarCell
        class="bar-btn"
        :label="props.isUnread ? $t('thread.actions.read') : $t('thread.actions.unread')"
        :title="props.isUnread ? $t('thread.actions.markRead') : $t('thread.actions.markUnread')"
        @click="emit('toggleRead')"
      >
        <template #icon>
          <AppIcon :icon="MailOpen" size="lg" v-if="props.isUnread" />
          <AppIcon :icon="Mail" size="lg" v-else />
        </template>
      </ToolbarCell>
      <ToolbarCell
        class="bar-btn"
        :label="$t('thread.actions.later')"
        :title="$t('thread.actions.snooze')"
        @click="emit('snooze')"
      >
        <template #icon>
          <AppIcon :icon="Clock" size="lg" />
        </template>
      </ToolbarCell>
    </div>
    <!-- A done thread gets the way back, not a Done that would do nothing. -->
    <AppButton
      v-if="props.isDone"
      class="bar-done"
      variant="primary"
      size="md"
      @click="emit('moveToInbox')"
    >
      <template #icon>
        <AppIcon :icon="Inbox" size="lg" class="bar-done__icon" />
      </template>
      {{ selectionCount ? $t('thread.selection.toInbox', { count: selectionCount }) : $t('commands.moveToInbox') }}
    </AppButton>
    <AppButton
      v-else
      class="bar-done"
      variant="primary"
      size="md"
      @click="emit('archive')"
    >
      <template #icon>
        <AppIcon :icon="Check" size="lg" class="bar-done__icon" />
      </template>
      {{ selectionCount ? $t('thread.selection.done', { count: selectionCount }) : $t('common.done') }}
    </AppButton>
  </div>
</template>

<style scoped>
/* ── Mobile bottom bar ── */
.mobile-bar {
  flex-shrink: 0;
  display: flex;
  align-items: stretch;
  gap: var(--space-xs);
  padding: var(--space-xs) var(--space-sm);
  background: var(--bg-surface);
  border-top: var(--border-default);
}

/* Each cell at least as wide as its label and a fingertip, the rest of the
   row shared out: "Antworten" and "Ungelesen" fit beside the pill at 360pt,
   with their own padding so neighbouring labels never run together. */
.mobile-bar-actions {
  display: grid;
  flex: 1 1 auto;
  min-width: 0;
  grid-auto-columns: minmax(max-content, 1fr);
  grid-auto-flow: column;
}

.mobile-bar-actions .bar-btn {
  min-width: var(--touch-target);
  min-height: 100%;
  padding-inline: var(--space-xs);
}

.mobile-bar .bar-done {
  position: relative;
  align-self: center;
  /* Prominent, but no wider than German "Erledigt" leaves room for at 360pt. */
  min-width: calc(2.5 * var(--touch-target));
  height: 56px;
  padding: 0 var(--space-md);
  overflow: hidden;
  gap: var(--space-sm);
  -webkit-tap-highlight-color: transparent;
  transition:
    transform var(--duration-fast) var(--ease-out),
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out),
    opacity var(--duration-fast) var(--ease-out);
}

.bar-done::before {
  content: '';
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: currentColor;
  opacity: 0;
  transition: opacity var(--duration-fast) linear;
}

.bar-done:active:not(:disabled) {
  transform: scale(0.98);
  border-radius: var(--radius-lg);
}

.bar-done:active:not(:disabled)::before {
  opacity: 0.12;
}

.bar-done:focus-visible::before {
  opacity: 0.1;
}

.bar-done:disabled {
  cursor: default;
  opacity: 0.38;
}

.bar-done__icon {
  position: relative;
  flex: 0 0 auto;
}

@media (prefers-reduced-motion: reduce) {
  .mobile-bar .bar-done,
  .bar-done::before {
    transition: none;
  }
}
</style>
