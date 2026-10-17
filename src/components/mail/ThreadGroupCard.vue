<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { SimilarThreadGroup } from '@/stores/threads';
import Avatar from '@/components/common/Avatar.vue';
import AvatarUnreadDot from '@/components/ui/AvatarUnreadDot.vue';
import CountBadge from '@/components/ui/CountBadge.vue';
import ListRowButton from '@/components/ui/ListRowButton.vue';
import ListRowMeta from '@/components/ui/ListRowMeta.vue';
import ThreadListQuickActions from './ThreadListQuickActions.vue';
import { useRowSwipe } from '@/composables/useRowSwipe';
import { useThreadActions } from '@/composables/useThreadActions';
import { formatListTime } from '@/utils/listTime';

const props = defineProps<{
  group: SimilarThreadGroup;
  busy?: boolean;
  targetKey?: string;
  keyboardFocused?: boolean;
}>();

const emit = defineEmits<{
  open: [groupId: string];
  done: [threadIds: string[]];
}>();

const { t } = useI18n();
const threadActions = useThreadActions();

const threadIds = computed(() => props.group.threads.map((thread) => thread.id));
const isUnread = computed(() => props.group.unreadCount > 0);

const {
  swipeTracking,
  swipeSettling,
  isTouchSwiping,
  swipeSurfaceStyle,
  swipeDirection,
  swipeIsLong,
  swipeCandidateSlot,
  swipeCandidateAction,
  swipeAction,
  swipeTone,
  swipeRevealStyle,
  swipeActionLabel,
  onSwipePointerDown,
  onSwipePointerMove,
  onSwipePointerEnd,
  onSwipePointerCancel,
  shouldSuppressClick,
} = useRowSwipe({
  enabled: () => !props.busy,
  actionLabel: (action) => {
    switch (action) {
      case 'toggle-read': return isUnread.value
        ? t('thread.actions.read')
        : t('thread.actions.unread');
      case 'done': return t('common.done');
      case 'pin': return t('thread.actions.pin');
      case 'aside': return t('thread.actions.aside');
      case 'trash': return t('common.delete');
      case 'none': return t('thread.actions.none');
      default: return '';
    }
  },
  onCommit: (action) => {
    const ids = threadIds.value;
    switch (action) {
      case 'toggle-read':
        void threadActions.setRead(ids, isUnread.value);
        break;
      case 'done':
        void threadActions.archive(ids);
        break;
      case 'pin':
        void threadActions.setPinned(ids, true);
        break;
      case 'aside':
        void threadActions.setAside(ids);
        break;
      case 'trash':
        void threadActions.trash(ids);
        break;
      case 'none':
        break;
    }
  },
});

const formattedTime = computed(() => formatListTime(props.group.lastMessageAt));

const bundleLabel = computed(() => t('thread.bundle.label', props.group.count));
const heading = computed(() => props.group.ciProject
  ? t('thread.bundle.ciProject', { project: props.group.ciProject })
  : props.group.senderName);

// Each layer represents a member, including stacks from a single sender.
const avatarLayers = computed(() => props.group.threads.slice(0, 3).map((thread) => ({
  id: thread.id,
  name: thread.participants?.[0]?.name || props.group.senderName,
  email: thread.participants?.[0]?.email || props.group.senderEmail,
})));
const subjectPreview = computed(() => {
  const subjects = [...new Set(props.group.threads.map((thread) => (thread.normalizedSubject ?? thread.subject)?.trim()).filter(Boolean))];
  return subjects.slice(0, 3).join(' · ') || props.group.title;
});

const cardLabel = computed(() =>
  [
    isUnread.value ? t('thread.actions.unread') : t('thread.actions.read'),
    heading.value,
    bundleLabel.value,
    subjectPreview.value,
    formattedTime.value,
  ].join(', '),
);

function openGroup() {
  if (shouldSuppressClick()) return;
  emit('open', props.group.id);
}

function onDragStart(event: DragEvent) {
  if (!event.dataTransfer) return;
  event.dataTransfer.setData('application/x-satchel-thread-ids', threadIds.value.join(','));
  event.dataTransfer.effectAllowed = 'move';
}
</script>

<template>
  <!-- Same gesture surface as a single row: swipe files the whole stack away,
       dragging drops it onto a sidebar folder. -->
  <div
    class="bundle-card group/bundle relative flex h-[var(--thread-row-h,72px)] w-full items-center px-2"
    role="presentation"
    :draggable="!isTouchSwiping"
    @dragstart="onDragStart"
    @pointerdown="onSwipePointerDown"
    @pointermove="onSwipePointerMove"
    @pointerup="onSwipePointerEnd"
    @pointercancel="onSwipePointerCancel"
  >
    <div
      class="bundle-card__swipe-track"
      :class="[
        `bundle-card__swipe-track--${swipeTone}`,
        swipeDirection ? `bundle-card__swipe-track--${swipeDirection}` : '',
      ]"
      aria-hidden="true"
    >
      <Transition name="swipe-action" mode="out-in">
        <span
          v-if="swipeCandidateAction && swipeDirection"
          :key="`${swipeDirection}-${swipeCandidateAction}-${swipeIsLong ? 'long' : 'short'}`"
          :data-swipe-action="swipeCandidateAction"
          :data-swipe-slot="swipeCandidateSlot"
          class="bundle-card__swipe-action"
          :class="[
            `bundle-card__swipe-action--${swipeDirection}`,
            { 'is-armed': swipeAction === swipeCandidateAction },
          ]"
          :style="swipeRevealStyle"
        >
          <span class="bundle-card__swipe-icon">
            <svg v-if="swipeCandidateAction === 'toggle-read'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="7.5" />
              <path v-if="isUnread" d="m8.7 12.2 2.1 2.1 4.6-4.7" />
            </svg>
            <svg v-else-if="swipeCandidateAction === 'done'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
              <path d="m5.5 12.4 4.1 4.1 8.9-9" />
            </svg>
            <svg v-else-if="swipeCandidateAction === 'pin'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
              <path d="M8 3.5h8l-1.1 5.8 3 3V14H6v-1.7l3-3L8 3.5Z" />
              <path d="M12 14v6.5" />
            </svg>
            <svg v-else-if="swipeCandidateAction === 'aside'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
              <path d="M5 7.5h14v10H5z" />
              <path d="M8 7.5V5h8v2.5M9 12h6" />
            </svg>
            <svg v-else-if="swipeCandidateAction === 'trash'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
              <path d="M4.5 7h15M9 7V4.5h6V7M7 7l.8 13h8.4L17 7M10 10.5v6M14 10.5v6" />
            </svg>
            <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
              <path d="M7 12h10" />
            </svg>
          </span>
          <span class="bundle-card__swipe-label">{{ swipeActionLabel }}</span>
        </span>
      </Transition>
    </div>

    <ListRowButton
      class="bundle-card__button"
      :disabled="props.busy"
      :class="[
        { 'bundle-card__button--keyboard-focused': props.keyboardFocused },
        swipeTracking ? 'bundle-card__button--swiping' : '',
        swipeSettling ? 'bundle-card__button--settling' : '',
      ]"
      :style="swipeSurfaceStyle"
      :data-list-target-key="props.targetKey"
      :aria-label="cardLabel"
      @click="openGroup"
    >
      <span
        class="bundle-avatar"
        aria-hidden="true"
      >
        <Avatar
          v-for="(layer, index) in avatarLayers"
          :key="layer.id"
          class="bundle-avatar__layer"
          :style="{ '--layer-index': index }"
          :name="layer.name"
          :email="layer.email"
          :size="28"
        />
        <AvatarUnreadDot v-if="isUnread" class="bundle-avatar__unread" />
      </span>

      <span class="bundle-card__content">
        <span class="bundle-card__heading">
          <span class="bundle-card__sender" :class="isUnread ? 'font-strong' : 'font-medium'">
            {{ heading }}
          </span>
          <CountBadge :value="props.group.count" :title="bundleLabel" />
        </span>
        <span class="bundle-card__subjects" :title="subjectPreview">{{ subjectPreview }}</span>
        <span class="bundle-card__summary">{{ bundleLabel }}</span>
      </span>
    </ListRowButton>

    <!-- Outermost slot, identical to the done quick action of single rows. -->
    <ListRowMeta class="bundle-card__meta" :time="formattedTime">
      <ThreadListQuickActions :bundle-count="props.group.count" :busy="props.busy" @action="emit('done', threadIds)" />
    </ListRowMeta>
  </div>
</template>

<style scoped>
.bundle-card {
  touch-action: pan-y pinch-zoom;
}

.bundle-card .bundle-card__button {
  position: relative;
  z-index: 1;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: var(--space-md);
  height: calc(100% - var(--space-xs));
  padding: 0 var(--space-md);
  background: var(--bg-stack);
  box-shadow: inset 0 0 0 1px var(--border-color-muted);
}

.bundle-card .bundle-card__button:hover:not(:disabled) {
  background: var(--bg-stack-hover);
}

.bundle-avatar {
  --avatar-dot-surface: var(--bg-stack);
  position: relative;
  display: block;
  width: 36px;
  height: 36px;
  flex-shrink: 0;
  border-radius: var(--radius-full);
}

.bundle-avatar .bundle-avatar__unread {
  top: var(--space-sm);
}

.bundle-card__button:hover .bundle-avatar {
  --avatar-dot-surface: var(--bg-stack-hover);
}

.bundle-avatar__layer {
  position: absolute;
  top: calc(var(--layer-index) * var(--space-xs));
  left: calc(var(--layer-index) * var(--space-xs));
  box-shadow: 0 0 0 2px var(--bg-stack);
}

.bundle-card__content {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: var(--space-2xs);
}

.bundle-card__heading {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: var(--space-1-5);
  padding-right: calc(2 * var(--space-2xl) + var(--space-xs));
}

.bundle-card__sender,
.bundle-card__subjects,
.bundle-card__summary {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.bundle-card__sender {
  min-width: 0;
  font-size: var(--font-size-md);
}

.bundle-card__subjects {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
}

.bundle-card__summary {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.bundle-card .bundle-card__button--keyboard-focused,
.bundle-card .bundle-card__button--keyboard-focused:hover:not(:disabled) {
  position: relative;
  z-index: 2;
  border-color: color-mix(in srgb, var(--accent-primary) 55%, var(--border-color));
  background: color-mix(in srgb, var(--accent-primary) 14%, var(--bg-list));
  box-shadow: var(--shadow-glow);
}

.bundle-card .bundle-card__meta {
  position: absolute;
  z-index: 2;
  right: var(--space-5);
  top: var(--space-md);
}

.bundle-card:hover,
.bundle-card:focus-within {
  --row-actions-visible: 1;
  --row-actions-events: auto;
}

/* Swipe surface mirrors the single-row beat: no transition while dragging,
   a short settle onto the commit offset, then the action fires. */
.bundle-card .bundle-card__button--swiping {
  transition: none;
}

.bundle-card .bundle-card__button--settling {
  transition: transform var(--duration-normal) var(--ease-out),
    background-color var(--duration-normal) var(--ease-out);
}

.bundle-card__swipe-track {
  position: absolute;
  inset: 8px 8px;
  z-index: 0;
  overflow: hidden;
  border-radius: var(--radius-md);
  background: var(--bg-inset);
  transition: background-color var(--duration-fast) var(--ease-out);
}

.bundle-card__swipe-track--blue { background: var(--swipe-blue); }
.bundle-card__swipe-track--green { background: var(--swipe-green); }
.bundle-card__swipe-track--amber { background: var(--swipe-amber); }
.bundle-card__swipe-track--cyan { background: var(--swipe-cyan); }
.bundle-card__swipe-track--red { background: var(--swipe-red); }
.bundle-card__swipe-track--neutral { background: var(--bg-inset); }

.bundle-card__swipe-action {
  --swipe-progress: 0;
  position: absolute;
  inset-block: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-width: 72px;
  max-width: 224px;
  overflow: hidden;
  padding-inline: 12px;
  color: var(--text-on-accent);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-bold);
  letter-spacing: 0.01em;
  opacity: calc(0.55 + var(--swipe-progress) * 0.45);
  transition: opacity var(--duration-fast) linear,
    width var(--duration-fast) linear;
}

.bundle-card__swipe-action--right {
  left: 0;
}

.bundle-card__swipe-action--left {
  right: 0;
  flex-direction: row-reverse;
}

.bundle-card__swipe-icon {
  display: grid;
  place-items: center;
  width: 31px;
  height: 31px;
  flex: none;
  border: 1.5px solid currentColor;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--text-on-accent) 8%, transparent);
  transition: background-color var(--duration-fast) var(--ease-out),
    transform var(--duration-normal) cubic-bezier(0.2, 0.85, 0.3, 1.28);
}

.bundle-card__swipe-icon svg {
  width: 17px;
  height: 17px;
}

.bundle-card__swipe-action.is-armed .bundle-card__swipe-icon {
  background: color-mix(in srgb, var(--text-on-accent) 20%, transparent);
  transform: scale(1.1);
}

.bundle-card__swipe-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.swipe-action-enter-active,
.swipe-action-leave-active {
  transition: opacity var(--duration-fast) var(--ease-out),
    transform var(--duration-fast) var(--ease-out);
}

.swipe-action-enter-from,
.swipe-action-leave-to {
  opacity: 0;
  transform: scale(.93);
}
</style>
