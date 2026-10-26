<script setup lang="ts">
import { ref } from 'vue';
import AppButton from '@/components/ui/AppButton.vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import IconButton from '@/components/ui/IconButton.vue';

const props = defineProps<{
  /** Keep the mobile actions mounted while the viewport changes. */
  visible: boolean;
  canReply: boolean;
  isUnread: boolean;
  isStarred: boolean;
  isAside: boolean;
  isDone: boolean;
}>();

const emit = defineEmits<{
  archive: [];
  moveToInbox: [];
  reply: [];
  toggleRead: [];
  snooze: [];
  togglePin: [];
  setAside: [];
  forward: [];
  delete: [];
}>();

const moreSheetOpen = ref(false);

function openMore() {
  moreSheetOpen.value = true;
}

function closeSheet() {
  moreSheetOpen.value = false;
}

type SheetAction = 'togglePin' | 'setAside' | 'forward' | 'delete';

function sheetAction(action: SheetAction) {
  closeSheet();
  if (action === 'togglePin') emit('togglePin');
  if (action === 'setAside') emit('setAside');
  if (action === 'forward') emit('forward');
  if (action === 'delete') emit('delete');
}
</script>

<template>
  <!-- ═══ MOBILE BOTTOM BAR ═══ -->
  <div v-if="props.visible" class="mobile-bar">
    <div class="mobile-bar-actions">
      <IconButton
        class="bar-btn"
        size="lg"
        :title="$t('common.more')"
        aria-haspopup="dialog"
        :aria-expanded="moreSheetOpen"
        @click="openMore"
      >
        <svg width="22" height="22" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
          <circle cx="3" cy="8" r="1.5" />
          <circle cx="8" cy="8" r="1.5" />
          <circle cx="13" cy="8" r="1.5" />
        </svg>
      </IconButton>
      <IconButton
        v-if="props.canReply"
        class="bar-btn"
        size="lg"
        :title="$t('common.reply')"
        @click="emit('reply')"
      >
        <svg width="22" height="22" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M6 3L1 8l5 5" />
          <path d="M1 8h9a5 5 0 015 5" />
        </svg>
      </IconButton>
      <IconButton
        class="bar-btn"
        size="lg"
        :title="props.isUnread ? $t('thread.actions.markRead') : $t('thread.actions.markUnread')"
        @click="emit('toggleRead')"
      >
        <svg v-if="props.isUnread" width="22" height="22" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M2 4.5h12v7a2 2 0 01-2 2H4a2 2 0 01-2-2z" />
          <path d="M2 5l6 4 6-4" />
        </svg>
        <svg v-else width="22" height="22" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M2 5.5l6 4 6-4" />
          <path d="M2 5.5V12a2 2 0 002 2h8a2 2 0 002-2V5.5" />
          <path d="M5 3.5h6" />
        </svg>
      </IconButton>
      <IconButton
        class="bar-btn"
        size="lg"
        :title="$t('thread.actions.snooze')"
        @click="emit('snooze')"
      >
        <svg width="22" height="22" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="8" cy="8" r="6" />
          <path d="M8 4v4l2.5 1.5" />
        </svg>
      </IconButton>
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
        <svg class="bar-done__icon" width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M8 11V3.4M5.4 8.4 8 11l2.6-2.6" />
          <path d="M2.8 11.2v1.2a1 1 0 0 0 1 1h8.4a1 1 0 0 0 1-1v-1.2" />
        </svg>
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
        <svg class="bar-done__icon" width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M3 8.5l3.5 3.5L13 5" />
        </svg>
      </template>
      {{ $t('common.done') }}
    </AppButton>
  </div>

  <!-- The sheet intentionally stays mounted and open across a viewport-mode change. -->
  <BottomSheet
    :label="$t('thread.toolbar.moreActions')"
    :open="moreSheetOpen"
    @dismiss="closeSheet"
  >
    <nav class="sheet-list" :aria-label="$t('thread.toolbar.moreActions')">
      <AppButton
        class="sheet-item"
        variant="ghost"
        @click="sheetAction('togglePin')"
      >
        <template #icon>
          <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M5.2 2.3h5.6l-.8 4 2.1 2.1v1.1H3.9V8.4L6 6.3l-.8-4Z" />
            <path d="M8 9.5v4.2" />
          </svg>
        </template>
        {{ props.isStarred ? $t('thread.actions.unpin') : $t('thread.actions.pin') }}
      </AppButton>
      <AppButton
        class="sheet-item"
        variant="ghost"
        @click="sheetAction('setAside')"
      >
        <template #icon>
          <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M2.5 5.5h11v7.5h-11z" />
            <path d="M5.5 8.5h5" />
            <path d="M8 1.8v4" />
            <path d="M6.2 4 8 5.8 9.8 4" />
          </svg>
        </template>
        {{ props.isAside ? $t('thread.actions.moveToInbox') : $t('thread.actions.setAside') }}
      </AppButton>
      <div class="sheet-divider" />
      <AppButton
        v-if="props.canReply"
        class="sheet-item"
        variant="ghost"
        @click="sheetAction('forward')"
      >
        <template #icon>
          <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M10 3l5 5-5 5" />
            <path d="M15 8H6a5 5 0 00-5 5" />
          </svg>
        </template>
        {{ $t('common.forward') }}
      </AppButton>
      <div v-if="props.canReply" class="sheet-divider" />
      <AppButton
        class="sheet-item sheet-item--danger"
        variant="ghost"
        @click="sheetAction('delete')"
      >
        <template #icon>
          <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M2 4h12M5 4V2.5A1.5 1.5 0 016.5 1h3A1.5 1.5 0 0111 2.5V4" />
            <path d="M13 4v9.5a1.5 1.5 0 01-1.5 1.5h-7A1.5 1.5 0 013 13.5V4" />
          </svg>
        </template>
        {{ $t('common.delete') }}
      </AppButton>
    </nav>
  </BottomSheet>
</template>

<style scoped>
/* ── Mobile bottom bar ── */
.mobile-bar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-sm) var(--space-md);
  background: var(--bg-surface);
  border-top: var(--border-default);
}

.mobile-bar-actions {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
}

.mobile-bar .bar-btn {
  width: 44px;
  height: 44px;
  min-width: 44px;
  min-height: 44px;
  border-radius: var(--radius-md);
  -webkit-tap-highlight-color: transparent;
}

.mobile-bar .bar-btn:active {
  background: var(--bg-hover);
}

.mobile-bar .bar-done {
  position: relative;
  min-width: 124px;
  height: 56px;
  padding: 0 var(--space-xl);
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
  stroke-width: 2.1;
}

/* ── Bottom-sheet action rows ── */
.sheet-list {
  display: flex;
  flex-direction: column;
  padding: var(--space-xs) var(--space-sm) var(--space-sm);
}

.sheet-list .sheet-item {
  justify-content: flex-start;
  width: 100%;
  min-height: 48px;
  gap: var(--space-md);
  padding: var(--space-md) var(--space-lg);
  border-radius: var(--radius-md);
  font-weight: var(--weight-regular);
  text-align: left;
  white-space: normal;
}

.sheet-list .sheet-item:active:not(:disabled) {
  background: var(--bg-hover);
}

.sheet-list .sheet-item svg {
  color: var(--text-secondary);
  flex-shrink: 0;
}

.sheet-list .sheet-item--danger {
  color: var(--accent-red);
}

.sheet-list .sheet-item--danger svg {
  color: var(--accent-red);
}

.sheet-divider {
  height: 1px;
  margin: var(--space-xs) var(--space-lg);
  background: var(--border-color-muted);
}

@media (prefers-reduced-motion: reduce) {
  .mobile-bar .bar-btn,
  .mobile-bar .bar-done,
  .bar-done::before {
    transition: none;
  }
}
</style>
