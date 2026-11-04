<script setup lang="ts">
/**
 * One action in a phone toolbar: icon over a short label, the whole cell the
 * target, as in an iOS toolbar or the app's TabBar. A bare icon left people
 * guessing what Mark and Snooze would do; the label says it.
 *
 * The visible label is the accessible name, so it stays short ("Unread",
 * "Later"); `title` may spell the action out. `primary` marks the one action
 * the bar is for in accent ink on a filled glyph, without making it a pill.
 */
withDefaults(
  defineProps<{
    label: string;
    title?: string;
    tone?: 'default' | 'primary';
    disabled?: boolean;
  }>(),
  {
    title: undefined,
    tone: 'default',
    disabled: false,
  },
);

defineEmits<{
  click: [event: MouseEvent];
}>();
</script>

<template>
  <button
    type="button"
    class="toolbar-cell"
    :class="`toolbar-cell--${tone}`"
    :title="title"
    :disabled="disabled"
    @click="$emit('click', $event)"
  >
    <span class="toolbar-cell__icon" aria-hidden="true"><slot name="icon" /></span>
    <span class="toolbar-cell__label">{{ label }}</span>
  </button>
</template>

<style scoped>
.toolbar-cell {
  display: grid;
  justify-items: center;
  align-content: center;
  gap: var(--space-2xs);
  min-width: 0;
  min-height: calc(var(--touch-target) + var(--space-sm));
  padding: var(--space-xs) var(--space-2xs);
  border: 0;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-medium);
  cursor: pointer;
  -webkit-tap-highlight-color: transparent;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.toolbar-cell__icon {
  display: grid;
  place-items: center;
  width: calc(var(--space-xl) + var(--space-sm));
  height: var(--space-xl);
  border-radius: var(--radius-full);
}

.toolbar-cell__label {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.toolbar-cell:active:not(:disabled) {
  background: var(--bg-hover);
}

.toolbar-cell--primary {
  color: var(--text-accent);
  font-weight: var(--weight-bold);
}

.toolbar-cell--primary .toolbar-cell__icon {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.toolbar-cell--primary .toolbar-cell__icon :deep(svg) {
  stroke-width: 2;
}

.toolbar-cell:disabled {
  opacity: 0.45;
  cursor: default;
}

@media (hover: hover) {
  .toolbar-cell:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .toolbar-cell--primary:hover:not(:disabled) {
    color: var(--text-accent);
  }
}
</style>
