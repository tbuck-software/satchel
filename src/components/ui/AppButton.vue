<script setup lang="ts">
import type { ShortcutHint } from '@/types/shortcuts';

const props = withDefaults(
  defineProps<{
    variant?: 'primary' | 'neutral' | 'soft' | 'secondary' | 'ghost' | 'danger' | 'link';
    size?: 'sm' | 'md';
    // A form with more than one field loses its implicit Enter submission when
    // the submit button is not part of it, so the kit has to be able to be one.
    type?: 'button' | 'submit';
    disabled?: boolean;
    title?: string;
    /** Shown in the tooltip and exposed as `aria-keyshortcuts`, never in the name. */
    shortcut?: ShortcutHint | null;
  }>(),
  {
    variant: 'soft',
    size: 'md',
    type: 'button',
    disabled: false,
    title: undefined,
    shortcut: null,
  },
);

defineEmits<{
  click: [event: MouseEvent];
}>();
</script>

<template>
  <button
    :type="type"
    class="app-button"
    :class="[`app-button--${variant}`, `app-button--${size}`]"
    :title="props.shortcut && title ? `${title} · ${props.shortcut.label}` : title"
    :aria-keyshortcuts="props.shortcut?.aria"
    :disabled="disabled"
    @click="$emit('click', $event)"
  >
    <span v-if="$slots.icon" class="app-button__icon">
      <slot name="icon" />
    </span>
    <slot />
  </button>
</template>

<style scoped>
.app-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-1-5);
  flex-shrink: 0;
  border: 0;
  border-radius: var(--radius-full);
  font-weight: var(--weight-semibold);
  white-space: nowrap;
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out),
    opacity var(--duration-fast) var(--ease-out);
}

.app-button--sm {
  height: 30px;
  padding: 0 var(--space-md);
  font-size: var(--font-size-xs);
}

.app-button--md {
  height: 36px;
  padding: 0 var(--space-lg);
  font-size: var(--font-size-sm);
}

.app-button__icon {
  display: inline-flex;
  align-items: center;
  margin-left: calc(-1 * var(--space-2xs));
}

.app-button--primary {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.app-button--neutral {
  background: var(--action-neutral);
  color: var(--action-neutral-ink);
}

.app-button--soft {
  background: var(--bg-raised);
  color: var(--text-primary);
}

/* The outlined twin of soft: same footprint, no fill. It replaces the
   hand-rolled `.secondary`/`.ghost` pairs that sat a few pixels off the
   primary next to them. */
.app-button--secondary {
  background: transparent;
  color: var(--text-primary);
  border: 1px solid var(--border-color);
}

.app-button--ghost {
  background: transparent;
  color: var(--text-secondary);
}

/* Inline actions share the surrounding text metrics and keep the kit's
   disabled, focus and keyboard behaviour. */
.app-button--link {
  height: auto;
  padding: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: inherit;
  white-space: normal;
}

.app-button--danger {
  background: color-mix(in srgb, var(--accent-red) 14%, var(--bg-raised));
  color: var(--accent-red);
}

.app-button:disabled {
  opacity: 0.45;
  cursor: default;
}

@media (prefers-reduced-motion: reduce) {
  .app-button {
    transition: none;
  }
}

/* A tap leaves WebKit's :hover on the button under the finger; the link
   variant kept its underline after the thread opened under it. */
@media (hover: hover) {
  .app-button--primary:hover:not(:disabled) {
    background: var(--accent-primary-hover);
  }

  .app-button--neutral:hover:not(:disabled) {
    background: color-mix(in srgb, var(--action-neutral) 90%, var(--bg-base));
  }

  .app-button--soft:hover:not(:disabled) {
    background: var(--bg-overlay);
  }

  .app-button--secondary:hover:not(:disabled) {
    background: var(--bg-hover);
  }

  .app-button--ghost:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .app-button--link:hover:not(:disabled) {
    text-decoration: underline;
  }

  .app-button--danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent-red) 24%, var(--bg-raised));
  }
}

/* Touch: the small pill is 30px; its hit area reaches 44px without the pill
   growing, so rows and headers keep their rhythm. */
@media (pointer: coarse) {
  .app-button--sm {
    position: relative;
  }

  .app-button--sm::after {
    content: '';
    position: absolute;
    inset: -7px 0;
  }
}
</style>
