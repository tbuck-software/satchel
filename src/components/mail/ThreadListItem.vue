<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import type { Thread } from '@/types/mail';
import { formatListTime, localeFormatter } from '@/utils/listTime';
import Avatar from '@/components/common/Avatar.vue';
import AvatarUnreadDot from '@/components/ui/AvatarUnreadDot.vue';
import ListRowMeta from '@/components/ui/ListRowMeta.vue';
import CountBadge from '@/components/ui/CountBadge.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import SenderFilingMenu from '@/components/mail/SenderFilingMenu.vue';
import ThreadListQuickActions from '@/components/mail/ThreadListQuickActions.vue';
import { useThreadActions } from '@/composables/useThreadActions';
import { useMessageLinks } from '@/composables/useMessageLinks';
import { useToast } from '@/composables/useToast';
import { useRowSwipe } from '@/composables/useRowSwipe';
import { useThreadsStore } from '@/stores/threads';
import { useTopicsStore } from '@/stores/topics';
import { useAccountsStore } from '@/stores/accounts';
import { useFollowUpsStore } from '@/stores/followUps';
import { isOwnEmail, participantDisplayName } from '@/utils/mailIdentity';
import { transportErrorLabel } from '@/utils/transportLabels';
import type { AnchorBox } from '@/utils/popoverPlacement';

const SnoozePicker = defineAsyncComponent(() => import('@/components/navigation/SnoozePicker.vue'));

defineOptions({ inheritAttrs: false });

const props = defineProps<{
  thread: Thread;
  active: boolean;
  selected: boolean;
  keyboardFocused?: boolean;
  selectionMode: boolean;
  /** 1-based position within the whole list (not just the rendered window). */
  posInSet?: number;
  /** Total number of rows in the list, for virtualised announcements. */
  setSize?: number;
  /**
   * The row sits on an already tinted card (priority card): no own surface
   * and a translucent hover, so the card's wash shows through.
   */
  tinted?: boolean;
}>();

const emit = defineEmits<{
  activate: [payload: { threadId: string; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean; keyboard?: boolean }];
  check: [payload: { threadId: string; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }];
}>();

const { t } = useI18n();
const threadActions = useThreadActions();
const messageLinks = useMessageLinks();
const threadsStore = useThreadsStore();
const topicsStore = useTopicsStore();
const accounts = useAccountsStore();
const followUps = useFollowUpsStore();
const toast = useToast();
const waitingPickerOpen = ref(false);
const followUpPickerOpen = ref(false);
const contextMenuOpen = ref(false);
const contextMenuAnchor = ref<AnchorBox | null>(null);
const senderMenuAnchor = ref<AnchorBox | null>(null);
/*
 * The menus mount on first use and stay for the row's lifetime, so closing
 * still plays their transition. Mounting both up front cost every visible row
 * two Teleports, two Transitions and their watchers for a menu that is rarely
 * opened.
 */
const contextMenuMounted = ref(false);
const senderMenuMounted = ref(false);
const rowRef = ref<HTMLElement | null>(null);

const primaryParticipantName = computed(() => {
  const participant = props.thread.participants[0];
  return participant
    ? participantDisplayName(accounts.accounts, participant)
    : t('common.unknown');
});
const primaryParticipantEmail = computed(() => props.thread.participants[0]?.email ?? '');
const primaryParticipantLabel = computed(() => {
  if (!isOwnEmail(accounts.accounts, primaryParticipantEmail.value)) {
    return primaryParticipantName.value;
  }
  const recipient = props.thread.participants.find(
    (participant) => !isOwnEmail(accounts.accounts, participant.email),
  );
  if (!recipient) return primaryParticipantName.value;
  return t('thread.row.senderToRecipient', {
    sender: primaryParticipantName.value,
    recipient: participantDisplayName(accounts.accounts, recipient),
  });
});
const isUnread = computed(() => props.thread.unreadCount > 0);
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
  enabled: () => !isLeaving.value && !props.selectionMode,
  actionLabel: (action) => {
    switch (action) {
      case 'toggle-read': return isUnread.value
        ? t('thread.actions.read')
        : t('thread.actions.unread');
      case 'done': return t('common.done');
      case 'pin': return props.thread.isStarred
        ? t('thread.actions.unpin')
        : t('thread.actions.pin');
      case 'aside': return props.thread.workflowState === 'aside'
        ? t('thread.actions.inbox')
        : t('thread.actions.aside');
      case 'trash': return t('common.delete');
      case 'none': return t('thread.actions.none');
      default: return '';
    }
  },
  onCommit: (action) => {
    switch (action) {
      case 'toggle-read':
        void threadActions.setRead([props.thread.id], isUnread.value);
        break;
      case 'done':
        void threadActions.archive([props.thread.id]);
        break;
      case 'pin':
        void threadActions.setPinned([props.thread.id], !props.thread.isStarred);
        break;
      case 'aside':
        if (props.thread.workflowState === 'aside') {
          void threadActions.moveToInbox([props.thread.id]);
        } else {
          void threadActions.setAside([props.thread.id]);
        }
        break;
      case 'trash':
        void threadActions.trash([props.thread.id]);
        break;
      case 'none':
        break;
    }
  },
});

/** Playing the check-off exit animation; removal from the list follows. */
const isLeaving = computed(() => threadsStore.leavingThreadIds.includes(props.thread.id));
const isCheckingOff = computed(
  () => threadsStore.leavingThreadStates[props.thread.id] === 'done',
);
const isFilingAside = computed(
  () => threadsStore.leavingThreadStates[props.thread.id] === 'aside',
);
const visibleSnippet = computed(() => props.thread.draftSnippet || props.thread.snippet);
const followUp = computed(() =>
  threadsStore.workflowFilter === 'waiting'
    ? followUps.forThread(props.thread.id)
    : null,
);
const followUpDue = computed(() =>
  followUp.value?.status === 'due'
    || Boolean(followUp.value?.due_at && new Date(followUp.value.due_at).getTime() <= Date.now()),
);
const followUpLabel = computed(() => {
  if (!followUp.value) return '';
  if (followUpDue.value) return t('thread.followUp.due');
  return t('thread.followUp.on', {
    date: localeFormatter('followUpDue', { day: '2-digit', month: 'short' })
      .format(new Date(followUp.value.due_at)),
  });
});

/**
 * Full spoken description of the row. Without this the unread state is
 * conveyed only by font weight and a decorative dot, and the subject/sender
 * are read as three unrelated fragments.
 */
const rowLabel = computed(() => {
  const parts = [
    isUnread.value ? t('thread.actions.unread') : t('thread.actions.read'),
    primaryParticipantLabel.value,
    (props.thread.normalizedSubject ?? props.thread.subject) || t('thread.row.noSubject'),
    formattedTime.value,
  ];
  if (props.thread.messageCount > 1) {
    parts.push(t('thread.row.messages', props.thread.messageCount));
  }
  if (props.thread.hasDraft) parts.push(t('thread.row.hasDraft'));
  if (props.thread.hasAttachments) parts.push(t('thread.row.hasAttachment'));
  if (props.thread.isStarred) parts.push(t('thread.row.pinned'));
  if (followUpLabel.value) parts.push(followUpLabel.value);
  return parts.join(', ');
});

const formattedTime = computed(() => formatListTime(props.thread.lastMessageAt));

function emitActivate(event: MouseEvent | KeyboardEvent) {
  if (shouldSuppressClick()) return;
  closeContextMenu();
  emit('activate', {
    threadId: props.thread.id,
    shiftKey: 'shiftKey' in event ? event.shiftKey : false,
    metaKey: 'metaKey' in event ? event.metaKey : false,
    ctrlKey: 'ctrlKey' in event ? event.ctrlKey : false,
    keyboard: 'key' in event,
  });
}

function openContextMenu(event: MouseEvent) {
  contextMenuAnchor.value = {
    left: event.clientX,
    top: event.clientY,
    right: event.clientX,
    bottom: event.clientY,
  };
  senderMenuAnchor.value = null;
  contextMenuMounted.value = true;
  contextMenuOpen.value = true;
}

function closeContextMenu() {
  senderMenuAnchor.value = null;
  contextMenuOpen.value = false;
}

function openSenderMenu(event: MouseEvent) {
  const trigger = (event.currentTarget as HTMLElement).getBoundingClientRect();
  senderMenuMounted.value = true;
  senderMenuAnchor.value = {
    left: trigger.left,
    top: trigger.top - 6,
    right: trigger.right,
    bottom: trigger.bottom,
  };
}

function targetThreadIds(): string[] {
  const selected = threadsStore.selectedThreadIds;
  return selected.includes(props.thread.id) ? [...selected] : [props.thread.id];
}

async function togglePinned() {
  closeContextMenu();
  await threadActions.setPinned(targetThreadIds(), !props.thread.isStarred);
}

async function toggleAside() {
  closeContextMenu();
  const ids = targetThreadIds();
  if (props.thread.workflowState === 'aside') await threadActions.moveToInbox(ids);
  else await threadActions.setAside(ids);
}

async function markDone() {
  closeContextMenu();
  await threadActions.archive(targetThreadIds());
}

async function toggleRead() {
  closeContextMenu();
  await threadActions.setRead(targetThreadIds(), isUnread.value);
}

function copyLink() {
  closeContextMenu();
  void messageLinks.copy(props.thread.id);
}

function addToTopic() {
  closeContextMenu();
  const selected = threadsStore.selectedThreadIds;
  const itemIds = selected.includes(props.thread.id) ? [...selected] : [props.thread.id];
  topicsStore.openPicker({ itemType: 'thread', itemIds });
}

async function toggleThreadPriority() {
  closeContextMenu();
  await threadActions.togglePriority(props.thread.id);
}

async function cancelFollowUp() {
  closeContextMenu();
  await followUps.cancel(props.thread.id);
}

async function applyFollowUp(date: string) {
  followUpPickerOpen.value = false;
  try {
    await followUps.schedule(props.thread.id, date);
    toast.success(t('thread.followUp.scheduled', {
      date: localeFormatter('followUpScheduled', { day: '2-digit', month: 'short' })
        .format(new Date(date)),
    }));
  } catch (err) {
    toast.error(t('thread.followUp.failed', { detail: transportErrorLabel(err) }));
  }
}

function quickAction(action: 'snooze' | 'done', event: MouseEvent) {
  event.stopPropagation();
  threadsStore.selectThread(props.thread.id);
  if (action === 'snooze') {
    waitingPickerOpen.value = true;
    return;
  }
  void markDone();
}

function applyWaiting(date: string) {
  waitingPickerOpen.value = false;
  void threadActions.snoozeAt(targetThreadIds(), date);
}

function onDragStart(event: DragEvent) {
  if (!event.dataTransfer) return;
  // If multiple selection includes this thread, drag all selected. Otherwise just this one.
  const selectedSet = new Set(threadsStore.selectedThreadIds);
  const ids = selectedSet.has(props.thread.id)
    ? Array.from(selectedSet)
    : [props.thread.id];
  event.dataTransfer.setData('application/x-satchel-thread-ids', ids.join(','));
  event.dataTransfer.effectAllowed = 'move';
}
</script>

<template>
  <article
    v-bind="$attrs"
    ref="rowRef"
    class="thread-row group relative h-[var(--thread-row-h,72px)] cursor-pointer select-none overflow-hidden rounded-xl text-left"
    :class="[
      isLeaving ? 'thread-row--leaving' : '',
      isCheckingOff ? 'thread-row--checking-off' : '',
      isFilingAside ? 'thread-row--filing-aside' : '',
      swipeTracking ? 'thread-row--swiping' : '',
      swipeSettling ? 'thread-row--settling' : '',
      props.keyboardFocused ? 'thread-row--keyboard-focused' : '',
      props.tinted ? 'thread-row--tinted' : '',
    ]"
    role="option"
    :aria-selected="props.selected || props.active"
    :aria-label="rowLabel"
    :aria-posinset="props.posInSet"
    :aria-setsize="props.setSize"
    :tabindex="props.active ? 0 : -1"
    :draggable="!isTouchSwiping"
    @dragstart="onDragStart"
    @click="emitActivate"
    @contextmenu.prevent.stop="openContextMenu"
    @pointerdown="onSwipePointerDown"
    @pointermove="onSwipePointerMove"
    @pointerup="onSwipePointerEnd"
    @pointercancel="onSwipePointerCancel"
    @keydown.enter.prevent="emitActivate"
    @keydown.space.prevent="emitActivate"
  >
    <div
      class="thread-row__swipe-track"
      :class="[
        `thread-row__swipe-track--${swipeTone}`,
        swipeDirection ? `thread-row__swipe-track--${swipeDirection}` : '',
      ]"
      aria-hidden="true"
    >
      <Transition name="swipe-action" mode="out-in">
        <span
          v-if="swipeCandidateAction && swipeDirection"
          :key="`${swipeDirection}-${swipeCandidateAction}-${swipeIsLong ? 'long' : 'short'}`"
          :data-swipe-action="swipeCandidateAction"
          :data-swipe-slot="swipeCandidateSlot"
          class="thread-row__swipe-action"
          :class="[
            `thread-row__swipe-action--${swipeDirection}`,
            { 'is-armed': swipeAction === swipeCandidateAction },
          ]"
          :style="swipeRevealStyle"
        >
          <span class="thread-row__swipe-icon">
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
          <span class="thread-row__swipe-label">{{ swipeActionLabel }}</span>
        </span>
      </Transition>
    </div>

    <div
      data-swipe-surface
      class="thread-row__surface relative grid h-full grid-cols-[auto_minmax(0,1fr)] items-center gap-3 rounded-xl px-3"
      :class="[
        props.keyboardFocused
          ? 'thread-row__surface--keyboard-focused'
          : props.selected
            ? 'bg-[color-mix(in_srgb,var(--accent-primary)_14%,var(--bg-list))]'
            : props.active
              ? 'thread-row__surface--open'
              : props.tinted
              ? 'bg-transparent group-hover:bg-[color-mix(in_srgb,var(--accent-amber)_9%,transparent)]'
              : 'bg-[var(--bg-list)] group-hover:bg-hover',
      ]"
      :style="swipeSurfaceStyle"
    >
    <span v-if="isCheckingOff" class="thread-row__done-cue" aria-hidden="true">
      <span class="thread-row__done-seal">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="m5.5 12.4 4.1 4.1 8.9-9" />
        </svg>
      </span>
    </span>

    <span
      class="thread-row__avatar relative h-9 w-9 shrink-0 rounded-full"
    >
      <Avatar
        :name="primaryParticipantName"
        :email="primaryParticipantEmail"
        :size="36"
        :class="props.thread.hasDraft ? 'brightness-[0.42] saturate-[0.8]' : ''"
      />
      <AvatarUnreadDot v-if="isUnread" />
      <svg
        v-if="props.thread.hasDraft"
        class="absolute inset-[9px] text-on-accent drop-shadow-[0_1px_2px_rgba(0,0,0,0.7)]"
        width="20"
        height="20"
        viewBox="0 0 20 20"
        fill="none"
        stroke="currentColor"
        stroke-width="1.65"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M13.8 3.2l3 3L7.1 15.9H4.2v-2.8L13.8 3.2Z" />
        <path d="M12.3 4.8l3 3" />
      </svg>
    </span>

    <div class="relative flex min-h-0 min-w-0 flex-col justify-center">
      <div class="thread-row-header grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-start gap-2">
        <span class="flex min-w-0 items-center gap-1.5">
          <span
            v-if="props.thread.hasDraft"
            class="shrink-0 text-sm font-strong text-accent-primary"
          >
            {{ $t('thread.row.draftPrefix') }}
          </span>
          <span
            class="block min-w-0 truncate text-md transition-colors"
            :class="isUnread ? 'font-strong text-primary' : props.active ? 'font-medium text-primary' : 'font-medium text-secondary'"
          >
            {{ primaryParticipantLabel }}
          </span>
        </span>
      </div>

      <div class="mt-0.5 flex min-w-0 items-center gap-1.5">
        <span
          class="min-w-0 truncate text-sm transition-colors"
          :class="
            isUnread
              ? 'font-medium text-primary'
              : props.active
                ? 'font-medium text-primary'
                : 'font-medium text-secondary'
          "
        >
          {{ props.thread.normalizedSubject ?? props.thread.subject }}
        </span>
        <CountBadge v-if="props.thread.messageCount > 1" :value="props.thread.messageCount" />
      </div>

      <div class="mt-0.5 flex min-w-0 items-center gap-2">
        <span
          v-if="followUp"
          class="follow-up-label"
          :class="{ 'follow-up-label--due': followUpDue }"
          :title="$d(new Date(followUp.due_at), 'long')"
        >
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="8" cy="8" r="5.5" />
            <path d="M8 5v3l2 1.5" />
          </svg>
          {{ followUpLabel }}
        </span>
        <span class="min-w-0 flex-1 truncate text-xs leading-[1.5] text-tertiary">
          {{ visibleSnippet }}
        </span>
        <svg
          v-if="props.thread.isStarred"
          class="shrink-0"
          style="color: var(--accent-yellow)"
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M5.2 2.3h5.6l-.8 4 2.1 2.1v1.1H3.9V8.4L6 6.3l-.8-4Z"/><path d="M8 9.5v4.2"/>
        </svg>
        <svg
          v-if="props.thread.isMuted"
          class="shrink-0 text-tertiary"
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M3 6v4h2.5l3 3V3l-3 3z"/>
          <path d="M11 5l3 3M14 5l-3 3"/>
        </svg>
        <svg
          v-if="props.thread.hasAttachments"
          class="shrink-0 text-tertiary"
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
        >
          <path d="M14 8.5L8 14.5a3.5 3.5 0 01-5-5L9.5 3a2.5 2.5 0 013.5 3.5L6.5 13a1.5 1.5 0 01-2-2.1L11 4.5" />
        </svg>
      </div>
      <!-- In the text column, so the time sits on the sender line at any row height. -->
      <ListRowMeta class="thread-row__meta" :time="formattedTime">
        <ThreadListQuickActions @action="quickAction" />
      </ListRowMeta>
    </div>
    <SnoozePicker
      v-if="waitingPickerOpen"
      :open="waitingPickerOpen"
      @choose="applyWaiting"
      @close="waitingPickerOpen = false"
    />
    <SnoozePicker
      v-if="followUpPickerOpen"
      :open="followUpPickerOpen"
      @choose="applyFollowUp"
      @close="followUpPickerOpen = false"
    />
    </div>
  </article>
  <ContextMenu
    v-if="contextMenuMounted"
    :open="contextMenuOpen"
    :anchor="contextMenuAnchor"
    :label="$t('thread.row.menuLabel')"
    @close="closeContextMenu"
  >
    <div class="menu-group" @mouseenter="senderMenuAnchor = null">
      <MenuItem icon="check" :label="$t('common.done')" @select="markDone" />
      <MenuItem
        icon="clock"
        :label="$t('thread.actions.snooze')"
        @select="waitingPickerOpen = true; closeContextMenu()"
      />
      <MenuItem
        :icon="props.thread.workflowState === 'aside' ? 'inbox' : 'aside'"
        :label="props.thread.workflowState === 'aside' ? $t('thread.actions.moveToInbox') : $t('thread.actions.setAside')"
        @select="toggleAside"
      />
      <MenuItem
        :icon="props.thread.isStarred ? 'unpin' : 'pin'"
        :label="props.thread.isStarred ? $t('thread.actions.unpinMenu') : $t('thread.actions.pin')"
        @select="togglePinned"
      />
      <MenuItem
        :icon="isUnread ? 'envelope-open' : 'envelope'"
        :label="isUnread ? $t('thread.actions.markRead') : $t('thread.actions.markUnread')"
        @select="toggleRead"
      />
      <MenuItem
        icon="bell"
        :label="followUp ? $t('thread.followUp.change') : $t('thread.followUp.set')"
        @select="followUpPickerOpen = true; closeContextMenu()"
      />
      <MenuItem v-if="followUp" icon="check" :label="$t('thread.followUp.done')" @select="cancelFollowUp" />
    </div>

    <div class="menu-group" @mouseenter="senderMenuAnchor = null">
      <MenuItem
        icon="bolt"
        :label="$t('thread.actions.priority')"
        :checked="props.thread.priority >= 2"
        @select="toggleThreadPriority"
      />
      <MenuItem icon="topic" :label="$t('thread.actions.addToTopic')" @select="addToTopic" />
      <MenuItem icon="link" :label="$t('thread.link.copy')" @select="copyLink" />
    </div>

    <div class="menu-group">
      <MenuItem
        icon="person"
        :label="$t('thread.row.sender')"
        submenu
        :expanded="senderMenuAnchor !== null"
        @mouseenter="openSenderMenu"
        @select="openSenderMenu"
      />
    </div>
  </ContextMenu>

  <ContextMenu
    v-if="senderMenuMounted"
    :open="contextMenuOpen && senderMenuAnchor !== null"
    :anchor="senderMenuAnchor"
    :label="$t('thread.row.senderMenuLabel', { email: primaryParticipantEmail })"
    nested
    :width="224"
    @close="senderMenuAnchor = null"
  >
    <SenderFilingMenu
      :account-id="props.thread.accountId"
      :email="primaryParticipantEmail"
      @close="closeContextMenu"
    />
  </ContextMenu>
</template>

<style scoped>
.follow-up-label {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 4px;
  max-width: 132px;
  overflow: hidden;
  color: var(--accent-amber);
  font-size: 11px;
  font-weight: var(--weight-strong);
  white-space: nowrap;
}

.follow-up-label--due {
  color: var(--accent-red);
}

.follow-up-label svg {
  flex: 0 0 auto;
}

.thread-row {
  touch-action: pan-y pinch-zoom;
  /* The pane it sits on, opaque, so the swipe track below stays hidden until
     the surface actually moves. */
  background: var(--bg-list);
}

.thread-row__surface--open .thread-row__avatar {
  --avatar-dot-surface: var(--bg-surface);
}

.thread-row__surface:hover .thread-row__avatar {
  --avatar-dot-surface: var(--bg-hover);
}

/* On the priority card the row is a see-through layer over the card's amber
   wash. The swipe track only goes clear while resting (tone `neutral`) — a
   real swipe still paints its solid action color. */
.thread-row--tinted,
.thread-row--tinted .thread-row__swipe-track--neutral {
  background: transparent;
}

/* The hover quick actions keep their raised elevation, warmed to the card —
   the neutral blue-grey `bg-raised` reads as a foreign object on amber. */
.thread-row--tinted .thread-row__meta :deep(button) {
  background: color-mix(in srgb, var(--accent-amber) 12%, var(--bg-raised));
}

.thread-row--tinted .thread-row__meta :deep(button:hover) {
  background: color-mix(in srgb, var(--accent-amber) 24%, var(--bg-raised));
}

.thread-row__surface {
  z-index: 2;
  transition: transform var(--duration-slow) var(--ease-out),
    background-color var(--duration-normal) var(--ease-out);
  will-change: transform;
}

/* The open thread is a sheet lifted off the list, the same treatment the
   active nav item gets. On paper a fill alone is a 1.07:1 step. */
.thread-row__surface--open {
  background: var(--bg-surface);
  box-shadow: var(--shadow-sm);
}

.thread-row__surface--keyboard-focused {
  background: color-mix(in srgb, var(--accent-primary) 16%, var(--bg-list));
  box-shadow:
    inset 0 0 0 1px color-mix(in srgb, var(--accent-primary) 44%, transparent),
    var(--shadow-glow);
}

.thread-row--keyboard-focused:focus-visible {
  outline: none;
}

.thread-row--swiping .thread-row__surface {
  transition: none;
}

.thread-row--settling .thread-row__surface {
  transition-duration: var(--duration-normal);
}

.thread-row__swipe-track {
  position: absolute;
  inset: 0;
  z-index: 1;
  overflow: hidden;
  border-radius: inherit;
  background: var(--bg-inset);
  transition: background-color var(--duration-fast) var(--ease-out);
}

.thread-row__swipe-track--blue { background: var(--swipe-blue); }
.thread-row__swipe-track--green { background: var(--swipe-green); }
.thread-row__swipe-track--amber { background: var(--swipe-amber); }
.thread-row__swipe-track--cyan { background: var(--swipe-cyan); }
.thread-row__swipe-track--red { background: var(--swipe-red); }
.thread-row__swipe-track--neutral { background: var(--bg-inset); }

.thread-row__swipe-action {
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
  font-size: 12px;
  font-weight: var(--weight-bold);
  letter-spacing: 0.01em;
  opacity: calc(0.55 + var(--swipe-progress) * 0.45);
  transition: opacity var(--duration-fast) linear,
    width var(--duration-fast) linear;
}

.thread-row__swipe-action--right {
  left: 0;
}

.thread-row__swipe-action--left {
  right: 0;
  flex-direction: row-reverse;
}

.thread-row__swipe-icon {
  display: grid;
  place-items: center;
  width: 31px;
  height: 31px;
  flex: none;
  border: 1.5px solid currentColor;
  border-radius: var(--radius-full);
  background: rgba(255, 255, 255, .08);
  transition: background-color var(--duration-fast) var(--ease-out),
    transform var(--duration-normal) cubic-bezier(0.2, 0.85, 0.3, 1.28);
}

.thread-row__swipe-icon svg {
  width: 17px;
  height: 17px;
}

.thread-row__swipe-action.is-armed .thread-row__swipe-icon {
  background: rgba(255, 255, 255, .2);
  transform: scale(1.1);
}

.thread-row__swipe-label {
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

/* Duration must match THREAD_LEAVE_MS in useWorkflow.ts. The first beat confirms
   the action; the second files the row away while ThreadGroupList closes its
   reserved virtual-list slot. */
.thread-row--leaving {
  pointer-events: none;
  transform-origin: 60% 50%;
  animation: thread-row-leave var(--duration-slow) cubic-bezier(0.32, 0, 0.15, 1) forwards;
}

.thread-row--checking-off {
  background: color-mix(in srgb, var(--accent-green) 11%, var(--bg-base));
}

.thread-row--checking-off .thread-row__surface {
  background: color-mix(in srgb, var(--accent-green) 11%, var(--bg-base));
}

.thread-row__done-cue {
  position: absolute;
  inset: 0;
  z-index: 4;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding-right: 16px;
  pointer-events: none;
  background: linear-gradient(
    90deg,
    transparent 42%,
    color-mix(in srgb, var(--accent-green) 13%, var(--bg-base)) 100%
  );
  opacity: 0;
  animation: thread-row-done-cue var(--duration-slow) cubic-bezier(0.2, 0.8, 0.2, 1) forwards;
}

.thread-row__done-seal {
  display: grid;
  place-items: center;
  width: 38px;
  height: 38px;
  border-radius: var(--radius-full);
  color: var(--accent-green);
  background: color-mix(in srgb, var(--accent-green) 20%, var(--bg-raised));
  box-shadow:
    0 0 0 1px color-mix(in srgb, var(--accent-green) 26%, transparent),
    0 8px 24px color-mix(in srgb, var(--accent-green) 18%, transparent);
  transform: scale(0.55) rotate(-8deg);
  animation: thread-row-done-seal var(--duration-slow) cubic-bezier(0.2, 0.8, 0.2, 1) forwards;
}

.thread-row__done-seal svg {
  width: 22px;
  height: 22px;
}

.thread-row__done-seal path {
  stroke-dasharray: 20;
  stroke-dashoffset: 20;
  animation: thread-row-done-check 170ms 45ms var(--ease-out) forwards;
}
@keyframes thread-row-leave {
  0%, 38% {
    opacity: 1;
    transform: translateX(0) scale(1);
  }
  to {
    opacity: 0;
    transform: translateX(34px) scale(0.985);
  }
}

@keyframes thread-row-leave-left {
  0%, 38% {
    opacity: 1;
    transform: translateX(0) scale(1);
  }
  to {
    opacity: 0;
    transform: translateX(-34px) scale(0.985);
  }
}

.thread-row--filing-aside {
  animation-name: thread-row-leave-left;
}

@keyframes thread-row-done-cue {
  0% { opacity: 0; }
  18%, 58% { opacity: 1; }
  100% { opacity: 0; transform: translateX(12px); }
}

@keyframes thread-row-done-seal {
  0% { opacity: 0; transform: scale(0.55) rotate(-8deg); }
  25%, 58% { opacity: 1; transform: scale(1) rotate(0); }
  100% { opacity: 0; transform: scale(0.92) translateX(8px); }
}

@keyframes thread-row-done-check {
  to { stroke-dashoffset: 0; }
}
.thread-row__meta {
  position: absolute;
  top: 0;
  right: 0;
}

.thread-row-header {
  padding-right: calc(2 * var(--space-2xl) + var(--space-xs));
}

.thread-row:hover,
.thread-row:focus-within {
  --row-actions-visible: 1;
  --row-actions-events: auto;
}
/* Inside the row only: the wrapper height comes from the virtualizer's
   constants, and a container query that moved it drifted the scrolling. */
@container (max-width: 360px) {
  .thread-row__surface { gap: 9px; padding-inline: 9px; }
  .thread-row-header { gap: 4px; }
}

@media (prefers-reduced-motion: reduce) {
  .thread-row--leaving,
  .thread-row__done-cue,
  .thread-row__done-seal,
  .thread-row__done-seal path {
    animation: none;
  }

  .thread-row--leaving {
    opacity: 0;
  }
}
</style>
