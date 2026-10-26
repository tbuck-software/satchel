<script setup lang="ts">
import type { ShortcutHint } from '@/types/shortcuts';

const props = withDefaults(
  defineProps<{
    title?: string;
    /** Shown in the tooltip and exposed as `aria-keyshortcuts`, never in the name. */
    shortcut?: ShortcutHint | null;
    size?: 'sm' | 'md' | 'lg';
    tone?: 'default' | 'accent' | 'danger' | 'warning';
    active?: boolean;
    disabled?: boolean;
  }>(),
  {
    title: undefined,
    shortcut: null,
    size: 'md',
    tone: 'default',
    active: false,
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
    class="icon-button"
    :class="[`icon-button--${size}`, `icon-button--${tone}`, { 'icon-button--active': active }]"
    :title="props.shortcut && title ? `${title} · ${props.shortcut.label}` : title"
    :aria-label="title"
    :aria-keyshortcuts="props.shortcut?.aria"
    :disabled="disabled"
    @click="$emit('click', $event)"
  >
    <slot />
  </button>
</template>

<style scoped>
.icon-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.icon-button--sm {
  width: 26px;
  height: 26px;
}

.icon-button--md {
  width: 32px;
  height: 32px;
}

.icon-button--lg {
  width: 38px;
  height: 38px;
}

.icon-button:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.icon-button--accent {
  color: var(--accent-primary);
}

.icon-button--accent:hover:not(:disabled) {
  color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 14%, transparent);
}

.icon-button--danger:hover:not(:disabled) {
  color: var(--accent-red);
  background: color-mix(in srgb, var(--accent-red) 12%, transparent);
}

.icon-button--warning {
  color: var(--accent-amber);
}

.icon-button--warning:hover:not(:disabled) {
  color: var(--accent-amber);
  background: color-mix(in srgb, var(--accent-amber) 12%, transparent);
}

.icon-button--active {
  background: color-mix(in srgb, var(--accent-primary) 16%, transparent);
  color: var(--accent-primary);
}

.icon-button:disabled {
  opacity: 0.45;
  cursor: default;
}

/* Grow the layout with the touch target so adjacent controls never overlap. */
@media (pointer: coarse) {
  .icon-button {
    min-width: var(--touch-target);
    min-height: var(--touch-target);
  }
}

@media (prefers-reduced-motion: reduce) {
  .icon-button {
    transition: none;
  }
}
</style>
