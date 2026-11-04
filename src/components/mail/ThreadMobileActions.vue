<script setup lang="ts">
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
}>();

const emit = defineEmits<{
  archive: [];
  moveToInbox: [];
  reply: [];
  toggleRead: [];
  snooze: [];
}>();
</script>

<template>
  <!-- ═══ MOBILE BOTTOM BAR ═══
       Three labelled cells, icon over a short label and the whole cell the
       target, beside the prominent Done pill. Everything else the thread
       offers is behind the ⋯ in its nav bar (ThreadMenuSheet.vue). -->
  <div v-if="props.visible" class="mobile-bar">
    <div class="mobile-bar-actions">
      <ToolbarCell
        v-if="props.canReply"
        class="bar-btn"
        :label="$t('common.reply')"
        @click="emit('reply')"
      >
        <template #icon>
          <AppIcon size="lg">
            <path d="M6 3L1 8l5 5" />
            <path d="M1 8h9a5 5 0 015 5" />
          </AppIcon>
        </template>
      </ToolbarCell>
      <ToolbarCell
        class="bar-btn"
        :label="props.isUnread ? $t('thread.actions.read') : $t('thread.actions.unread')"
        :title="props.isUnread ? $t('thread.actions.markRead') : $t('thread.actions.markUnread')"
        @click="emit('toggleRead')"
      >
        <template #icon>
          <AppIcon size="lg" v-if="props.isUnread">
            <path d="M2 4.5h12v7a2 2 0 01-2 2H4a2 2 0 01-2-2z" />
            <path d="M2 5l6 4 6-4" />
          </AppIcon>
          <AppIcon size="lg" v-else>
            <path d="M2 5.5l6 4 6-4" />
            <path d="M2 5.5V12a2 2 0 002 2h8a2 2 0 002-2V5.5" />
            <path d="M5 3.5h6" />
          </AppIcon>
        </template>
      </ToolbarCell>
      <ToolbarCell
        class="bar-btn"
        :label="$t('thread.actions.later')"
        :title="$t('thread.actions.snooze')"
        @click="emit('snooze')"
      >
        <template #icon>
          <AppIcon size="lg">
            <circle cx="8" cy="8" r="6" />
            <path d="M8 4v4l2.5 1.5" />
          </AppIcon>
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
        <AppIcon size="lg" class="bar-done__icon">
          <path d="M8 11V3.4M5.4 8.4 8 11l2.6-2.6" />
          <path d="M2.8 11.2v1.2a1 1 0 0 0 1 1h8.4a1 1 0 0 0 1-1v-1.2" />
        </AppIcon>
      </template>
      {{ $t('commands.moveToInbox') }}
    </AppButton>
    <AppButton
      v-else
      class="bar-done"
      variant="primary"
      size="md"
      @click="emit('archive')"
    >
      <template #icon>
        <AppIcon size="lg" class="bar-done__icon">
          <path d="M3 8.5l3.5 3.5L13 5" />
        </AppIcon>
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
