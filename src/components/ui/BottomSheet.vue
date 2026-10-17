<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useUiStore } from '@/stores/ui';

/**
 * The small, transient surface: a sort menu, a set of quick actions.
 *
 * Narrow, it rises from the bottom edge, where a thumb already is — a popover
 * anchored to a control at the top of a phone screen is a stretch away and
 * often lands half off-screen. Wide, it stays the anchored popover it should
 * be, because a pointer has no reach problem and a sheet would be theatre.
 *
 * Deliberately *not* a ModalShell, and it stays that way now that the shell
 * traps focus: these are dismissable glances, not places. A sheet answers a
 * flick downwards and a tap anywhere else, and locking the keyboard inside it
 * would be the wrong promise. Anything the user can get lost inside belongs in
 * a modal with its own back step instead. Only the scrim is shared, through
 * --bg-backdrop.
 */
const props = withDefaults(defineProps<{
  label: string;
  open: boolean;
  /** Width of the centred desktop panel. Mobile sheets always fill the viewport. */
  desktopWidth?: string;
}>(), {
  desktopWidth: '420px',
});

const emit = defineEmits<{ dismiss: [] }>();
const ui = useUiStore();
const panel = ref<HTMLElement | null>(null);
const dragOffset = ref(0);
const dragging = ref(false);
const settling = ref(false);

interface DragGesture {
  lastX: number;
  lastY: number;
  totalX: number;
  totalY: number;
  lastAt: number;
  velocityY: number;
}

let gesture: DragGesture | null = null;
let dragged = false;
let removeOverlayRegistration: (() => void) | null = null;

const panelStyle = computed(() => ({
  '--sheet-drag-y': `${dragOffset.value}px`,
  '--sheet-desktop-width': props.desktopWidth,
}));

const scrimStyle = computed(() => {
  if (!dragging.value || dragOffset.value === 0) return undefined;
  const height = panel.value?.getBoundingClientRect().height || 400;
  return { opacity: String(Math.max(0.15, 1 - dragOffset.value / height)) };
});

function dismiss() {
  emit('dismiss');
}

function beginDrag(clientX: number, clientY: number, timeStamp: number) {
  if (!ui.isSinglePane) return;
  gesture = {
    lastX: clientX,
    lastY: clientY,
    totalX: 0,
    totalY: 0,
    lastAt: timeStamp,
    velocityY: 0,
  };
  dragged = false;
  settling.value = false;
}

function moveDrag(clientX: number, clientY: number, timeStamp: number, event: Event) {
  if (!gesture || !panel.value) return;

  const deltaX = clientX - gesture.lastX;
  const deltaY = clientY - gesture.lastY;
  const elapsed = Math.max(1, timeStamp - gesture.lastAt);
  gesture.totalX += deltaX;
  gesture.totalY += deltaY;
  gesture.lastX = clientX;
  gesture.lastY = clientY;
  gesture.lastAt = timeStamp;

  if (!dragging.value) {
    if (Math.abs(gesture.totalX) > Math.abs(gesture.totalY)) {
      gesture = null;
      return;
    }
    // Upward movement and movement inside already-scrolled content remain
    // native scrolling. Once the content is at its top, a downward pull owns
    // the gesture no matter where on the sheet it began.
    if (gesture.totalY < 4 || panel.value.scrollTop > 0 || deltaY <= 0) return;
    dragging.value = true;
  }

  event.preventDefault();
  dragOffset.value = Math.max(0, dragOffset.value + deltaY);
  gesture.velocityY = deltaY / elapsed;
  dragged ||= dragOffset.value > 6;
}

function finishDrag() {
  if (!gesture) return;
  const height = panel.value?.getBoundingClientRect().height || 400;
  const distanceThreshold = Math.min(120, Math.max(72, height * 0.22));
  const shouldDismiss = dragging.value
    && (dragOffset.value >= distanceThreshold
      || (dragOffset.value > 24 && gesture.velocityY > 0.55));

  gesture = null;
  dragging.value = false;
  if (shouldDismiss) {
    dismiss();
    return;
  }

  if (dragOffset.value > 0) settling.value = true;
  dragOffset.value = 0;
}

function cancelDrag() {
  gesture = null;
  dragging.value = false;
  settling.value = dragOffset.value > 0;
  dragOffset.value = 0;
}

function onTouchStart(event: TouchEvent) {
  if (event.touches.length !== 1) return;
  // Length was just verified to be exactly one.
  const touch = event.touches[0]!;
  beginDrag(touch.clientX, touch.clientY, event.timeStamp);
}

function onTouchMove(event: TouchEvent) {
  if (event.touches.length !== 1) return;
  const touch = event.touches[0]!;
  moveDrag(touch.clientX, touch.clientY, event.timeStamp, event);
}

function onPointerDown(event: PointerEvent) {
  if (event.pointerType === 'touch' || event.button !== 0) return;
  beginDrag(event.clientX, event.clientY, event.timeStamp);
}

function onPointerMove(event: PointerEvent) {
  if (event.pointerType === 'touch') return;
  const wasDragging = dragging.value;
  moveDrag(event.clientX, event.clientY, event.timeStamp, event);
  if (wasDragging || !dragging.value) return;

  // Capture only after a drag starts. Capturing on pointerdown redirects
  // ordinary button clicks to the panel, including on desktop.
  try {
    panel.value?.setPointerCapture?.(event.pointerId);
  } catch {
    // The drag remains usable while the pointer stays over the panel.
  }
}

function suppressDraggedClick(event: MouseEvent) {
  if (!dragged) return;
  event.preventDefault();
  event.stopPropagation();
  dragged = false;
}

watch(() => props.open, (open) => {
  removeOverlayRegistration?.();
  removeOverlayRegistration = null;
  if (open) {
    removeOverlayRegistration = ui.registerOverlay({
      isOpen: () => props.open,
      dismiss,
    });
    return;
  }
  cancelDrag();
}, { immediate: true });

onBeforeUnmount(() => removeOverlayRegistration?.());
</script>

<template>
  <Transition :name="ui.isSinglePane ? 'sheet-rise' : 'sheet-fade'">
    <div v-if="open" class="sheet-root" :class="{ 'sheet-root--anchored': !ui.isSinglePane }">
      <div class="sheet-scrim" :style="scrimStyle" @click="dismiss" />
      <div
        ref="panel"
        class="sheet-panel"
        :class="{
          'sheet-panel--dragging': dragging,
          'sheet-panel--settling': settling,
        }"
        :style="panelStyle"
        role="dialog"
        :aria-label="label"
        @click.capture="suppressDraggedClick"
        @touchstart="onTouchStart"
        @touchmove="onTouchMove"
        @touchend="finishDrag"
        @touchcancel="cancelDrag"
        @pointerdown="onPointerDown"
        @pointermove="onPointerMove"
        @pointerup="finishDrag"
        @pointercancel="cancelDrag"
        @transitionend="settling = false"
      >
        <!-- Grab handle only where the gesture exists to match it. -->
        <div v-if="ui.isSinglePane" class="sheet-grip" aria-hidden="true" />
        <slot />
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.sheet-root {
  position: fixed;
  inset: 0;
  z-index: 200;
  display: flex;
  align-items: flex-end;
  justify-content: center;
}

.sheet-scrim {
  position: absolute;
  inset: 0;
  background: var(--bg-backdrop);
}

.sheet-panel {
  position: relative;
  width: 100%;
  max-height: 70vh;
  padding-right: var(--satchel-safe-area-right);
  padding-bottom: var(--satchel-safe-area-bottom);
  padding-left: var(--satchel-safe-area-left);
  border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  background: var(--bg-surface);
  box-shadow: var(--shadow-lg);
  overflow: auto;
  overscroll-behavior-y: contain;
  transform: translateY(var(--sheet-drag-y, 0));
}

.sheet-panel--dragging {
  cursor: grabbing;
  user-select: none;
}

.sheet-panel--settling {
  transition: transform var(--duration-fast) var(--ease-out);
}

.sheet-grip {
  width: 36px;
  height: 4px;
  margin: var(--space-sm) auto var(--space-xs);
  border-radius: 2px;
  background: var(--border-color);
}

/* Wide: an ordinary centred popover, no scrim theatre. */
.sheet-root--anchored {
  align-items: center;
}

.sheet-root--anchored .sheet-panel {
  width: auto;
  min-width: 260px;
  max-width: var(--sheet-desktop-width);
  border-radius: var(--radius-lg);
}

.sheet-rise-enter-active .sheet-panel,
.sheet-rise-leave-active .sheet-panel {
  transition: transform var(--duration-normal) var(--ease-out);
}

.sheet-rise-enter-from .sheet-panel,
.sheet-rise-leave-to .sheet-panel {
  transform: translateY(100%);
}

.sheet-fade-enter-active,
.sheet-fade-leave-active {
  transition: opacity var(--duration-fast) var(--ease-out);
}

.sheet-fade-enter-from,
.sheet-fade-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .sheet-panel--settling,
  .sheet-rise-enter-active .sheet-panel,
  .sheet-rise-leave-active .sheet-panel,
  .sheet-fade-enter-active,
  .sheet-fade-leave-active {
    transition: none;
  }
}
</style>
