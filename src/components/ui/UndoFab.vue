<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useUndoFab, type UndoFabEntry } from '@/composables/useUndoFab';
import { useShortcutHints } from '@/composables/useShortcutHints';
import AppIcon from '@/components/ui/AppIcon.vue';

const props = defineProps<{
  /**
   * Both variants are viewport-fixed in the bottom-left corner, like a FAB.
   * floating: phone corner (mirrors the compose FAB's lane on the right).
   * docked:   desktop, aligned to the left edge of the mail list column.
   */
  variant: 'floating' | 'docked';
}>();

const { entries, run } = useUndoFab();
const { t } = useI18n();
const { hint } = useShortcutHints();

/** Newest entry is the one the button acts on. */
const activeIndex = ref(Math.max(0, entries.value.length - 1));
watch(
  () => [entries.value.at(-1)?.key, entries.value.at(-1)?.label, entries.value.length],
  () => {
    activeIndex.value = Math.max(0, entries.value.length - 1);
  },
);
const activeEntry = computed<UndoFabEntry | null>(() => entries.value[activeIndex.value] ?? null);

function cycle() {
  if (entries.value.length < 2) return;
  activeIndex.value = (activeIndex.value + 1) % entries.value.length;
}

async function runActive() {
  const entry = activeEntry.value;
  if (!entry) return;
  await run(entry);
}

// The ring runs on the toast's hard wall-clock deadline, so it must not be
// animated with a CSS duration of its own — it tracks time that belongs to
// the undo window, not to a transition.
const now = ref(Date.now());
let frame: number | null = null;

function tick() {
  now.value = Date.now();
  frame = requestAnimationFrame(tick);
}

onMounted(() => { frame = requestAnimationFrame(tick); });
onBeforeUnmount(() => { if (frame !== null) cancelAnimationFrame(frame); });

const RING_RADIUS = 15;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

const fraction = computed(() => {
  const entry = activeEntry.value;
  if (!entry) return 0;
  const remaining = (entry.expiresAt - now.value) / entry.durationMs;
  return Math.max(0, Math.min(1, remaining));
});
const secondsLeft = computed(() => {
  const entry = activeEntry.value;
  if (!entry) return 0;
  return Math.max(0, Math.ceil((entry.expiresAt - now.value) / 1000));
});

let reducedMotionQuery: MediaQueryList | null = null;
const reducedMotion = ref(false);
onMounted(() => {
  reducedMotionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
  reducedMotion.value = reducedMotionQuery.matches;
  reducedMotionQuery.addEventListener('change', onReducedMotionChange);
});
function onReducedMotionChange() {
  reducedMotion.value = reducedMotionQuery?.matches ?? false;
}
onBeforeUnmount(() => reducedMotionQuery?.removeEventListener('change', onReducedMotionChange));

const ringStyle = computed(() => {
  // Reduced motion gets a static full ring; the countdown stays readable
  // through the title/aria text instead of an animation.
  if (reducedMotion.value) return {};
  return { strokeDashoffset: `${RING_CIRCUMFERENCE * (1 - fraction.value)}` };
});

const label = computed(() => activeEntry.value?.label ?? '');
/** The undo key acts on the newest receipt, so only that one shows it. */
const undoKey = computed(() =>
  activeEntry.value && activeEntry.value.key === entries.value.at(-1)?.key ? hint('undo') : null,
);
</script>

<template>
  <div
    v-if="activeEntry"
    class="undo-fab"
    :class="`undo-fab--${props.variant}`"
  >
    <button
      type="button"
      class="undo-fab__main"
      :title="t('ui.undoTitle', { label, seconds: secondsLeft })"
      :aria-label="t('ui.undoAria', { label, seconds: secondsLeft })"
      :aria-keyshortcuts="undoKey?.aria"
      @click="void runActive()"
    >
      <svg class="undo-fab__ring" viewBox="0 0 36 36" aria-hidden="true">
        <circle class="undo-fab__ring-track" cx="18" cy="18" :r="RING_RADIUS" />
        <circle
          class="undo-fab__ring-value"
          cx="18"
          cy="18"
          :r="RING_RADIUS"
          :stroke-dasharray="RING_CIRCUMFERENCE"
          :style="ringStyle"
        />
      </svg>
      <AppIcon class="undo-fab__icon">
        <path d="M3.5 6.5h6a3.2 3.2 0 010 6.4H6" />
        <path d="M6 3.5L3 6.5l3 3" />
      </AppIcon>
    </button>

    <button
      v-if="entries.length > 1"
      type="button"
      class="undo-fab__queue"
      :title="t('ui.toastQueue', { count: entries.length })"
      :aria-label="t('ui.toastQueueAria', { count: entries.length })"
      @click="cycle"
    >
      {{ entries.length }}
    </button>

    <!-- Desktop only: the phone has no keyboard to name, and no room. The
         live region in ToastStack announces it; this is for the eyes. -->
    <span v-if="props.variant === 'docked'" class="undo-fab__receipt" aria-hidden="true">
      <span class="undo-fab__label">{{ label }}</span>
      <span v-if="undoKey" class="undo-fab__key">
        <kbd>{{ undoKey.label }}</kbd>
      </span>
    </span>
  </div>
</template>

<style scoped>
.undo-fab {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
  /* Above the toast stack: both live in the bottom lane, and a simultaneous
     error toast must not swallow the undo control. */
  z-index: 1001;
}

.undo-fab--floating {
  position: fixed;
  /* Mirrors the compose FAB on the right: same lane, other side. */
  left: calc(20px + var(--satchel-safe-area-left));
  bottom: calc(20px + var(--satchel-safe-area-bottom));
}

.undo-fab--docked {
  position: fixed;
  /* Pinned to the very bottom, aligned with the mail list column. */
  left: calc(var(--sidebar-w, 0px) + var(--space-lg, 20px));
  bottom: 20px;
}

.undo-fab__main {
  position: relative;
  display: grid;
  place-items: center;
  width: 44px;
  height: 44px;
  padding: 0;
  border: var(--border-default);
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
  color: var(--text-primary);
  box-shadow: var(--shadow-md);
  cursor: pointer;
  transition:
    transform var(--duration-fast) var(--ease-out),
    background-color var(--duration-fast) var(--ease-out);
}

.undo-fab__main:hover {
  transform: translateY(-1px);
  background: color-mix(in srgb, var(--bg-overlay) 88%, var(--text-primary));
}

.undo-fab__main:active {
  transform: translateY(0);
}

.undo-fab__ring {
  position: absolute;
  /* Explicit full-size box: the 36-unit viewBox scales uniformly into it,
     which keeps the circle concentric with the button at any density. */
  inset: 3px;
  width: calc(100% - 6px);
  height: calc(100% - 6px);
  transform: rotate(-90deg);
}

.undo-fab__ring circle {
  fill: none;
  stroke-width: 2.4;
}

/* Theme-adaptive: near-white on dark, near-black on light. */
.undo-fab__ring-track {
  stroke: color-mix(in srgb, var(--text-primary) 20%, transparent);
}

.undo-fab__ring-value {
  stroke: var(--text-primary);
  stroke-linecap: round;
}

.undo-fab__icon {
  width: 18px;
  height: 18px;
}

.undo-fab__queue {
  display: grid;
  place-items: center;
  flex: 0 0 22px;
  width: 22px;
  height: 22px;
  padding: 0;
  border: var(--border-default);
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
  cursor: pointer;
}

.undo-fab__receipt {
  display: inline-flex;
  align-items: center;
  gap: var(--space-xs);
  max-width: 320px;
  padding: var(--space-2xs) var(--space-xs) var(--space-2xs) var(--space-sm);
  border: var(--border-default);
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
  box-shadow: var(--shadow-md);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.undo-fab__label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.undo-fab__key kbd {
  display: inline-grid;
  place-items: center;
  min-width: 20px;
  height: 20px;
  padding: 0 var(--space-2xs);
  border: var(--border-default);
  border-radius: var(--radius-xs);
  background: var(--bg-raised);
  color: var(--text-primary);
  font-family: var(--font-mono);
  font-size: var(--font-size-2xs);
}

@media (prefers-reduced-motion: reduce) {
  .undo-fab__main,
  .undo-fab__main:hover {
    transition: none;
    transform: none;
  }
}
</style>
