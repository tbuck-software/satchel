<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useToast, type Toast, type ToastAction } from '@/composables/useToast';
import { useShortcutHints } from '@/composables/useShortcutHints';
import {
  clearanceAbove,
  toastClearanceRevision,
  toastObstacleRects,
} from '@/composables/useToastClearance';
import IconButton from '@/components/ui/IconButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';

const toast = useToast();
const { t } = useI18n();
const { hint } = useShortcutHints();
const isCompact = ref(false);
const compactActiveId = ref<string | null>(null);
let compactQuery: MediaQueryList | null = null;
const updateCompact = () => {
  isCompact.value = compactQuery?.matches ?? false;
};
onMounted(() => {
  compactQuery = window.matchMedia('(max-width: 640px)');
  updateCompact();
  compactQuery.addEventListener('change', updateCompact);
});
onBeforeUnmount(() => compactQuery?.removeEventListener('change', updateCompact));

interface PresentedToast extends Toast {
  sourceIds: string[];
  sourceActions: ToastAction[];
}

const presentedToasts = computed<PresentedToast[]>(() => {
  const presented: PresentedToast[] = [];
  for (const item of toast.state.toasts) {
    // Undo actions flagged for the FAB are presented there, not as a row
    // stacked above the bottom bar — one affordance per surface.
    if (item.action?.present === 'fab') continue;
    const groupIndex = item.groupKey
      ? presented.findIndex((candidate) => candidate.groupKey === item.groupKey)
      : -1;
    if (groupIndex < 0) {
      presented.push({
        ...item,
        message: item.groupMessage?.(item.count ?? 1) ?? item.message,
        sourceIds: [item.id],
        sourceActions: item.action ? [item.action] : [],
      });
      continue;
    }

    // Move an updated group to the newest position while keeping one visual
    // row. Every source toast stays in state with its own hard Undo deadline.
    // splice at a found index always yields exactly the moved group.
    const group = presented.splice(groupIndex, 1)[0]!;
    group.sourceIds.push(item.id);
    if (item.action) group.sourceActions.push(item.action);
    group.count = (group.count ?? 1) + (item.count ?? 1);
    group.message = item.groupMessage?.(group.count) ?? item.message;
    presented.push(group);
  }
  return presented;
});

const compactActionableToasts = computed(() =>
  presentedToasts.value.filter((item) => item.action),
);
watch(
  () => compactActionableToasts.value.map((item) => item.id).join('|'),
  (ids, previousIds) => {
    if (ids !== previousIds) compactActiveId.value = compactActionableToasts.value.at(-1)?.id ?? null;
  },
);

const visibleToasts = computed(() => {
  if (!isCompact.value) return presentedToasts.value;
  const latestActionable = compactActionableToasts.value.find(
    (item) => item.id === compactActiveId.value,
  ) ?? compactActionableToasts.value.at(-1);
  const latestError = [...presentedToasts.value].reverse().find((item) => item.tone === 'error');
  // Undo has a hard provider deadline, so it stays reachable; an error must
  // not expire unseen behind it either. Those two may share the screen, and
  // nothing else: the navigator reaches older actions without more rows.
  if (latestActionable && latestError && latestError.id !== latestActionable.id) {
    return [latestError, latestActionable];
  }
  const latest = latestActionable ?? latestError ?? presentedToasts.value.at(-1);
  return latest ? [latest] : [];
});

/*
 * Lift the stack above the composer's send row and the reply bar while a
 * toast is up (see `useToastClearance`). Measured on the events that move
 * those surfaces — a toast arriving, a resize, the end of a drag or a
 * transition, an owner's refresh — rather than every frame: an error toast
 * can stay for a long time and a frame loop would run all of it.
 */
const stackRef = ref<HTMLElement | null>(null);
const clearance = ref(0);
/** `--space-sm`, between the stack and the surface it clears. */
const CLEARANCE_GAP = 8;
let clearanceFrame: number | null = null;

function measureClearance() {
  clearanceFrame = null;
  const stack = stackRef.value;
  if (!stack || visibleToasts.value.length === 0) {
    clearance.value = 0;
    return;
  }
  const rect = stack.getBoundingClientRect();
  // Measured from where the stack sits when it is not lifted.
  const resting = {
    top: rect.top + clearance.value,
    bottom: rect.bottom + clearance.value,
    left: rect.left,
    right: rect.right,
  };
  const lift = clearanceAbove(resting, toastObstacleRects(), CLEARANCE_GAP);
  // Never past the top edge: covering a surface beats a toast nobody can see.
  clearance.value = Math.max(0, Math.min(lift, resting.top - CLEARANCE_GAP));
}

function scheduleClearance() {
  if (clearanceFrame !== null || typeof requestAnimationFrame === 'undefined') return;
  clearanceFrame = requestAnimationFrame(measureClearance);
}

const CLEARANCE_TRIGGERS = ['pointerup', 'transitionend'] as const;
let listening = false;

function listenForClearance(active: boolean) {
  if (active === listening || typeof window === 'undefined') return;
  listening = active;
  if (active) {
    window.addEventListener('resize', scheduleClearance);
    for (const name of CLEARANCE_TRIGGERS) document.addEventListener(name, scheduleClearance, true);
  } else {
    window.removeEventListener('resize', scheduleClearance);
    for (const name of CLEARANCE_TRIGGERS) document.removeEventListener(name, scheduleClearance, true);
  }
}

watch(
  [() => visibleToasts.value.map((item) => item.id).join('|'), toastClearanceRevision],
  async ([ids]) => {
    listenForClearance(ids !== '');
    await nextTick();
    scheduleClearance();
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  listenForClearance(false);
  if (clearanceFrame !== null) cancelAnimationFrame(clearanceFrame);
});

/*
 * Announcements go through two regions that exist before any toast does.
 * A live region inserted together with its text, as the rows are, is often
 * not read at all by VoiceOver. Undo receipts shown by the FAB land here too.
 */
const politeAnnouncement = ref('');
const assertiveAnnouncement = ref('');
const announcedIds = new Set<string>();
let announceTimer: number | null = null;

function announcementFor(item: Toast): string {
  const count = item.groupKey
    ? toast.state.toasts
      .filter((candidate) => candidate.groupKey === item.groupKey)
      .reduce((total, candidate) => total + (candidate.count ?? 1), 0)
    : item.count ?? 1;
  const message = item.groupMessage?.(count) ?? item.message;
  if (item.action?.present !== 'fab') return message;
  const key = hint('undo')?.label;
  return key ? t('ui.undoAnnouncement', { label: message, key }) : message;
}

watch(
  () => toast.state.toasts.map((item) => item.id).join('|'),
  () => {
    const current = new Set(toast.state.toasts.map((item) => item.id));
    for (const id of announcedIds) if (!current.has(id)) announcedIds.delete(id);
    const fresh = toast.state.toasts.filter((item) => !announcedIds.has(item.id));
    fresh.forEach((item) => announcedIds.add(item.id));
    const newest = fresh.at(-1);
    if (!newest) return;
    const target = newest.tone === 'error' ? assertiveAnnouncement : politeAnnouncement;
    // Cleared first, so a repeat of the same text is still a change.
    target.value = '';
    if (announceTimer !== null) window.clearTimeout(announceTimer);
    announceTimer = window.setTimeout(() => {
      announceTimer = null;
      target.value = announcementFor(newest);
    }, 100);
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (announceTimer !== null) window.clearTimeout(announceTimer);
});

function dismissPresented(item: PresentedToast) {
  item.sourceIds.forEach((id) => toast.dismiss(id));
}

async function runPresentedAction(item: PresentedToast) {
  await toast.runActions(item.sourceIds, item.sourceActions);
}

function showPreviousCompactUndo() {
  const actions = compactActionableToasts.value;
  if (actions.length < 2) return;
  const currentIndex = actions.findIndex((item) => item.id === visibleToasts.value[0]?.id);
  const previousIndex = currentIndex <= 0 ? actions.length - 1 : currentIndex - 1;
  compactActiveId.value = actions[previousIndex]?.id ?? compactActiveId.value;
}
</script>

<template>
  <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">{{ politeAnnouncement }}</div>
  <div class="sr-only" role="alert" aria-live="assertive" aria-atomic="true">{{ assertiveAnnouncement }}</div>
  <div ref="stackRef" class="stack" :style="{ '--toast-clearance': `${clearance}px` }">
    <TransitionGroup name="toast" tag="div" class="stack__inner">
      <div
        v-for="item in visibleToasts"
        :key="item.id"
        class="toast"
        :class="`toast--${item.tone}`"
        @mouseenter="item.sourceIds.forEach(toast.pause)"
        @mouseleave="item.sourceIds.forEach(toast.resume)"
        @focusin="item.sourceIds.forEach(toast.pause)"
        @focusout="item.sourceIds.forEach(toast.resume)"
      >
        <span class="toast__icon" aria-hidden="true">
          <AppIcon v-if="item.tone === 'success'">
            <path d="m3 8.4 3.2 3.2L13 4.8" />
          </AppIcon>
          <AppIcon v-else-if="item.tone === 'error'">
            <circle cx="8" cy="8" r="5.8" />
            <path d="M8 5v3.6" />
            <path d="M8 10.8h.01" />
          </AppIcon>
          <AppIcon v-else-if="item.tone === 'warning'">
            <path d="M8 2.4 14.2 13H1.8L8 2.4Z" />
            <path d="M8 6.6v3" />
            <path d="M8 11.6h.01" />
          </AppIcon>
          <AppIcon v-else>
            <circle cx="8" cy="8" r="5.8" />
            <path d="M8 7.4v3.4" />
            <path d="M8 5.2h.01" />
          </AppIcon>
        </span>

        <span class="toast__message">{{ item.message }}</span>

        <button
          v-if="item.action"
          type="button"
          class="toast__action"
          @click="void runPresentedAction(item)"
        >
          {{ item.action.label }}
        </button>

        <button
          v-if="isCompact && compactActionableToasts.length > 1"
          type="button"
          class="toast__queue"
          :title="$t('ui.toastQueue', { count: compactActionableToasts.length })"
          @click="showPreviousCompactUndo"
        >
          {{ compactActionableToasts.length }}
        </button>

        <IconButton size="sm" :title="$t('ui.dismissToast')" @click="dismissPresented(item)">
          <AppIcon size="sm">
            <path d="M4 4l8 8M12 4l-8 8" />
          </AppIcon>
        </IconButton>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.stack {
  position: fixed;
  right: calc(var(--space-xl) + var(--satchel-safe-area-right));
  /* Not transitioned: the lift is measured from the stack's own position. */
  bottom: calc(var(--space-xl) + var(--satchel-safe-area-bottom) + var(--toast-clearance, 0px));
  z-index: 1000;
  max-width: min(
    440px,
    calc(
      100vw - 2 * var(--space-xl) - var(--satchel-safe-area-left) -
        var(--satchel-safe-area-right)
    )
  );
  pointer-events: none;
}

.stack__inner {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--space-sm);
}

.toast {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding: var(--space-2-5) var(--space-2-5) var(--space-2-5) var(--space-md);
  border-radius: var(--radius-md);
  border: var(--border-default);
  background: var(--bg-overlay);
  box-shadow: var(--shadow-md);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}

/* The tone is carried by the icon, not by a stripe that reads as decoration. */
.toast__icon {
  display: grid;
  place-items: center;
  flex-shrink: 0;
  width: 20px;
  height: 20px;
  border-radius: var(--radius-full);
}

.toast__icon svg {
  width: 13px;
  height: 13px;
}

.toast--info .toast__icon {
  background: color-mix(in srgb, var(--accent-primary) 18%, transparent);
  color: var(--accent-primary);
}

.toast--success .toast__icon {
  background: color-mix(in srgb, var(--accent-green) 18%, transparent);
  color: var(--accent-green);
}

.toast--warning .toast__icon {
  background: color-mix(in srgb, var(--accent-amber) 18%, transparent);
  color: var(--accent-amber);
}

.toast--error .toast__icon {
  background: color-mix(in srgb, var(--accent-red) 18%, transparent);
  color: var(--accent-red);
}

.toast--error {
  border-color: color-mix(in srgb, var(--accent-red) 38%, var(--border-color));
}

.toast__message {
  flex: 1;
  min-width: 0;
  line-height: 1.45;
}

.toast__action {
  flex-shrink: 0;
  height: 26px;
  padding: 0 var(--space-2-5);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--accent-primary);
  font: inherit;
  font-weight: var(--weight-semibold);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.toast__action:hover {
  background: color-mix(in srgb, var(--accent-primary) 14%, transparent);
}

.toast__queue {
  display: grid;
  place-items: center;
  flex: 0 0 26px;
  width: 26px;
  height: 26px;
  padding: 0;
  border: var(--border-default);
  border-radius: var(--radius-full);
  background: var(--bg-secondary);
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
  cursor: pointer;
}

.toast-enter-active,
.toast-leave-active {
  transition:
    transform var(--duration-normal) var(--ease-out),
    opacity var(--duration-normal) var(--ease-out);
}

.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(10px);
}

/* Keeps the survivors from jumping when one in the middle disappears. */
.toast-leave-active {
  position: absolute;
}

.toast-move {
  transition: transform var(--duration-normal) var(--ease-out);
}

@media (max-width: 640px) {
  .stack {
    right: var(--space-md);
    left: var(--space-md);
    /* Clear both the 56px compose FAB and the thread detail action bar. */
    bottom: calc(92px + var(--satchel-safe-area-bottom) + var(--toast-clearance, 0px));
    max-width: none;
  }

  .stack__inner {
    align-items: stretch;
  }

  .toast {
    min-height: 56px;
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
  }
}

@media (prefers-reduced-motion: reduce) {
  .toast-enter-active,
  .toast-leave-active,
  .toast-move {
    transition: none;
  }
}
</style>
