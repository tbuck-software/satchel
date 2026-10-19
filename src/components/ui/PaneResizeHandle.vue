<script setup lang="ts">
import { onBeforeUnmount } from 'vue';

/**
 * The divider between two panes: drag it, or focus it and use the arrow keys.
 *
 * It was a mouse-only `role="separator"`: not focusable, no keys, no value,
 * so a keyboard or screen-reader user could neither reach nor read it. The
 * limits come from the caller (the UI store owns them); the handle only
 * reports the width it wants and the store clamps it.
 */
const props = defineProps<{
  label: string;
  /** The width of the pane the handle resizes, in px. */
  value: number;
  min: number;
  max: number;
}>();

const emit = defineEmits<{
  resize: [width: number];
}>();

/** Arrow-key step; Shift takes the large one. */
const STEP = 16;
const LARGE_STEP = 64;

let startX = 0;
let startWidth = 0;

function onPointerMove(event: MouseEvent) {
  emit('resize', startWidth + event.clientX - startX);
}

function stopDrag() {
  document.body.style.cursor = '';
  document.body.style.userSelect = '';
  window.removeEventListener('mousemove', onPointerMove);
  window.removeEventListener('mouseup', stopDrag);
}

function startDrag(event: MouseEvent) {
  startX = event.clientX;
  startWidth = props.value;
  document.body.style.cursor = 'col-resize';
  document.body.style.userSelect = 'none';
  window.addEventListener('mousemove', onPointerMove);
  window.addEventListener('mouseup', stopDrag);
}

function onKeydown(event: KeyboardEvent) {
  const step = event.shiftKey ? LARGE_STEP : STEP;
  let next: number | null = null;
  if (event.key === 'ArrowLeft') next = props.value - step;
  else if (event.key === 'ArrowRight') next = props.value + step;
  else if (event.key === 'Home') next = props.min;
  else if (event.key === 'End') next = props.max;
  if (next === null) return;
  event.preventDefault();
  // Keep the global single-key shortcuts out of it.
  event.stopPropagation();
  emit('resize', Math.min(props.max, Math.max(props.min, next)));
}

onBeforeUnmount(stopDrag);
</script>

<template>
  <div
    class="pane-resize-handle"
    role="separator"
    tabindex="0"
    aria-orientation="vertical"
    :aria-label="label"
    :aria-valuenow="value"
    :aria-valuemin="min"
    :aria-valuemax="max"
    @mousedown.prevent="startDrag"
    @keydown="onKeydown"
  />
</template>

<style scoped>
.pane-resize-handle {
  width: 1px;
  position: relative;
  cursor: col-resize;
  background: var(--border-color-muted);
  z-index: 2;
}

/* A 7px grab area around the hairline. */
.pane-resize-handle::before {
  content: '';
  position: absolute;
  top: 0;
  bottom: 0;
  left: -3px;
  width: 7px;
}

.pane-resize-handle:hover,
.pane-resize-handle:active,
.pane-resize-handle:focus-visible {
  background: var(--accent-primary);
  box-shadow: 0 0 8px color-mix(in srgb, var(--accent-primary) 40%, transparent);
}

/* The shared ring (STYLEGUIDE.md, Focus): base.css only covers form controls and links. */
.pane-resize-handle:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}
</style>
