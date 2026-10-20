<script setup lang="ts">
/**
 * Floating menu anchored to a click point or a trigger element. Used for the
 * thread row's right-click menu, its sender flyout and the sender popover
 * behind the avatar — all three want the same dismissal, clamping and arrow
 * behaviour, and drifted apart while each rolled its own.
 *
 * Nested menus (`nested`) skip the backdrop: the outer menu already owns it,
 * and a second one would swallow the click that opened the flyout.
 */
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { useUiStore } from '@/stores/ui';
import { placeMenu, type AnchorBox, type MenuPlacement } from '@/utils/popoverPlacement';

const props = withDefaults(
  defineProps<{
    open: boolean;
    /** Viewport box of the trigger; a click point is a zero-size box. */
    anchor: AnchorBox | null;
    label: string;
    width?: number;
    nested?: boolean;
    /** Move focus to the first item on open. Defaults to on for top-level menus. */
    focusFirst?: boolean;
  }>(),
  { width: 232, nested: false, focusFirst: undefined },
);

const emit = defineEmits<{ close: [] }>();

const ui = useUiStore();
const menuRef = ref<HTMLElement | null>(null);
const placement = ref<MenuPlacement | null>(null);
let returnFocusTo: HTMLElement | null = null;
let removeOverlay: (() => void) | null = null;

const style = computed(() => ({
  left: placement.value?.left ?? '-9999px',
  top: placement.value?.top ?? '-9999px',
  maxHeight: placement.value?.maxHeight ?? 'none',
  width: `${props.width}px`,
}));

function items(): HTMLElement[] {
  return Array.from(menuRef.value?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ?? [])
    .filter((item) => !item.hasAttribute('disabled'));
}

function focusItem(offset: number, from?: HTMLElement | null) {
  const all = items();
  if (all.length === 0) return;
  const current = from ?? (document.activeElement as HTMLElement | null);
  const index = current ? all.indexOf(current) : -1;
  const next = index < 0
    ? (offset > 0 ? 0 : all.length - 1)
    : (index + offset + all.length) % all.length;
  all[next]?.focus();
}

function onKeydown(event: KeyboardEvent) {
  switch (event.key) {
    case 'Escape':
      event.stopPropagation();
      emit('close');
      break;
    case 'ArrowRight': {
      const current = document.activeElement as HTMLElement | null;
      if (current?.getAttribute('aria-haspopup') !== 'menu') break;
      event.preventDefault();
      current.click();
      break;
    }
    case 'ArrowLeft':
      if (!props.nested) break;
      event.preventDefault();
      emit('close');
      break;
    case 'ArrowDown':
      event.preventDefault();
      focusItem(1);
      break;
    case 'ArrowUp':
      event.preventDefault();
      focusItem(-1);
      break;
    case 'Home':
      event.preventDefault();
      items()[0]?.focus();
      break;
    case 'End':
      event.preventDefault();
      items().at(-1)?.focus();
      break;
    default:
      break;
  }
}

/** Measure first, place second — a menu clamped against a guessed height
 *  either floats off the edge or opens upward for no reason. */
async function reposition() {
  if (!props.open || !props.anchor) return;
  await nextTick();
  const element = menuRef.value;
  if (!element) return;
  placement.value = placeMenu(
    props.anchor,
    { width: window.innerWidth, height: window.innerHeight },
    { width: props.width, height: element.scrollHeight + 12 },
    { gap: props.nested ? 4 : 0 },
  );
}

watch(
  () => [props.open, props.anchor] as const,
  ([open]) => {
    if (!open) {
      placement.value = null;
      // Hand the keyboard back to whoever opened the menu, unless focus has
      // already moved on to something the chosen item opened.
      if (menuRef.value?.contains(document.activeElement)) {
        returnFocusTo?.focus({ preventScroll: true });
      }
      returnFocusTo = null;
      return;
    }
    // A flyout re-anchored by another trigger returns to that trigger.
    const active = document.activeElement as HTMLElement | null;
    if (!menuRef.value?.contains(active)) returnFocusTo = active;
    void reposition().then(() => {
      if (props.focusFirst ?? !props.nested) items()[0]?.focus({ preventScroll: true });
    });
  },
  { immediate: true },
);

// Registered while open only: rows mount a menu each, and the back stack
// orders surfaces by registration.
watch(
  () => props.open,
  (open) => {
    if (open && !removeOverlay) {
      removeOverlay = ui.registerOverlay({ isOpen: () => props.open, dismiss: () => emit('close') });
    } else if (!open && removeOverlay) {
      removeOverlay();
      removeOverlay = null;
    }
  },
  { immediate: true },
);

function onViewportChange() {
  void reposition();
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      window.addEventListener('resize', onViewportChange);
      window.addEventListener('scroll', onViewportChange, true);
    } else {
      window.removeEventListener('resize', onViewportChange);
      window.removeEventListener('scroll', onViewportChange, true);
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  window.removeEventListener('resize', onViewportChange);
  window.removeEventListener('scroll', onViewportChange, true);
  removeOverlay?.();
});
</script>

<template>
  <Teleport to="body">
    <div
      v-if="props.open && !props.nested"
      class="context-menu-backdrop"
      aria-hidden="true"
      @pointerdown="emit('close')"
      @contextmenu.prevent="emit('close')"
    />
    <!-- `appear`: rows mount their menu on first open, already open. -->
    <Transition name="context-menu" appear>
      <div
        v-if="props.open"
        ref="menuRef"
        class="context-menu"
        :class="{ 'context-menu--nested': props.nested }"
        role="menu"
        :aria-label="props.label"
        :style="style"
        @click.stop
        @keydown="onKeydown"
      >
        <slot />
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.context-menu-backdrop {
  position: fixed;
  inset: 0;
  z-index: 999;
}

.context-menu {
  position: fixed;
  z-index: 1000;
  display: grid;
  gap: var(--space-2xs);
  overflow-y: auto;
  padding: var(--space-1-5);
  border: var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  box-shadow: var(--shadow-lg);
  overscroll-behavior: contain;
}

.context-menu--nested {
  z-index: 1001;
}

/* Groups are separated by air, not by rules — a 12-item menu with four
   hairlines reads as a form. */
.context-menu :deep(.menu-group + .menu-group) {
  margin-top: var(--space-1-5);
}

.context-menu-enter-active {
  transition:
    opacity var(--duration-fast) var(--ease-out),
    transform var(--duration-fast) var(--ease-out);
}

.context-menu-enter-from {
  opacity: 0;
  transform: scale(0.97);
}

@media (prefers-reduced-motion: reduce) {
  .context-menu-enter-active {
    transition: none;
  }
}
</style>
