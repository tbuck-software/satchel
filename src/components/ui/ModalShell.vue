<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { useFocusTrap } from '@/composables/useFocusTrap';
import { useUiStore } from '@/stores/ui';

/**
 * The frame every full modal shares.
 *
 * Wide, it is a dialog: centred, bounded, floating over a dimmed page. Narrow,
 * it *is* the page — full bleed, no radius, no shadow, no gutter. The padding
 * that reads as breathing room on a desktop is space taken away from content
 * on a phone, where there is none to spare.
 *
 * The breakpoint is the same one that splits list from detail, so a modal and
 * the screen underneath never disagree about how much room they think there is.
 *
 * It also carries what a modal owes a keyboard and a screen reader —
 * `useFocusTrap` for the keyboard, plus `aria-modal` and a page that no longer
 * scrolls behind it. Built once here rather than eleven times badly.
 *
 * Escape is *not* handled here. It runs through `ui.dismissTopmostOverlay()`,
 * which knows the nesting order; a second handler on the frame would close the
 * wrong surface whenever two are open.
 */
withDefaults(
  defineProps<{
    label: string;
    /** Width of the dialog in the desktop case. The gutter is the frame's. */
    width?: string;
    /** Height of the dialog in the desktop case. `auto` follows the content. */
    height?: string;
    /** Whether clicking the backdrop dismisses. Off for destructive flows. */
    dismissOnBackdrop?: boolean;
  }>(),
  { width: '860px', height: '620px', dismissOnBackdrop: true },
);

const emit = defineEmits<{ dismiss: [] }>();
const ui = useUiStore();
const panel = ref<HTMLElement | null>(null);

// Focus moved in, kept in, and handed back — the same one every other overlay
// uses. A dialog rests on its own panel so the name is announced first.
const { onTab } = useFocusTrap(panel, { restingFocus: 'container' });

function onBackdropClick(event: MouseEvent) {
  if (event.target === event.currentTarget) emit('dismiss');
}

// Counted, because a modal can open on top of a modal and the inner one must
// not hand scrolling back while the outer one is still up.
let scrollLocks = 0;
let scrollBefore = '';

function lockScroll() {
  if (typeof document === 'undefined') return;
  if (scrollLocks === 0) {
    scrollBefore = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
  }
  scrollLocks += 1;
}

function unlockScroll() {
  if (typeof document === 'undefined') return;
  scrollLocks = Math.max(0, scrollLocks - 1);
  if (scrollLocks === 0) document.body.style.overflow = scrollBefore;
}

onMounted(lockScroll);
onBeforeUnmount(unlockScroll);
</script>

<template>
  <div
    class="shell-backdrop"
    :class="{ 'shell-backdrop--full': ui.isSinglePane }"
    @click="dismissOnBackdrop ? onBackdropClick($event) : undefined"
    @keydown.tab="onTab"
  >
    <div
      ref="panel"
      class="shell-panel"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      :aria-label="label"
      :style="ui.isSinglePane ? undefined : { '--shell-w': width, '--shell-h': height }"
    >
      <slot />
    </div>
  </div>
</template>

<style scoped>
.shell-backdrop {
  position: fixed;
  inset: 0;
  /* One layer for every modal: above the floating composer (240), below the
     toasts and the external-link warning that may need to sit on top of one. */
  z-index: 250;
  display: flex;
  align-items: center;
  justify-content: center;
  padding:
    var(--satchel-safe-area-top)
    var(--satchel-safe-area-right)
    var(--satchel-safe-area-bottom)
    var(--satchel-safe-area-left);
  background: var(--bg-backdrop);
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
}

/* Nothing shows through a full-screen modal, so the blur is only cost. */
.shell-backdrop--full {
  padding: 0;
  background: var(--bg-base);
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
}

.shell-panel {
  position: relative;
  display: flex;
  flex-direction: column;
  width: var(--shell-w);
  height: var(--shell-h);
  max-width: calc(100vw - 2 * var(--space-2xl));
  max-height: calc(100vh - 2 * var(--space-2xl));
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  box-shadow: var(--shadow-lg), 0 0 0 1px var(--border-color-muted);
  overflow: hidden;
}

.shell-backdrop--full .shell-panel {
  width: 100%;
  height: 100%;
  max-width: none;
  max-height: none;
  padding:
    var(--satchel-safe-area-top)
    var(--satchel-safe-area-right)
    var(--satchel-safe-area-bottom)
    var(--satchel-safe-area-left);
  border-radius: 0;
  box-shadow: none;
}
</style>
