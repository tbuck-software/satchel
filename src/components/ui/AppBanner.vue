<script setup lang="ts">
/**
 * A standing condition, not an event: something is wrong and stays wrong until
 * someone fixes it. Toasts are the wrong home for these — they leave on a
 * timer, and the problem does not.
 *
 * It sits in the layout flow rather than over it, so it pushes the app down
 * instead of covering the window chrome.
 *
 * `card` is the same notice as a self-contained block: rounded, inset from
 * its surroundings, text over actions. The strip is for a pane's own content
 * (a composer, a settings page); the shell's standing notices are cards, in
 * the sidebar footer or the phone's status sheet, never a band across the
 * top of the window.
 *
 * A danger banner announces itself: `status` waits for a pause in the screen
 * reader's queue, and a failure the user is about to work around cannot wait.
 */
import AppIcon from '@/components/ui/AppIcon.vue';
withDefaults(
  defineProps<{
    tone?: 'danger' | 'warning' | 'info';
    title: string;
    detail?: string;
    variant?: 'strip' | 'card';
  }>(),
  {
    tone: 'warning',
    detail: undefined,
    variant: 'strip',
  },
);
</script>

<template>
  <div
    class="banner"
    :class="[`banner--${tone}`, `banner--${variant}`]"
    :role="tone === 'danger' ? 'alert' : 'status'"
  >
    <span class="banner__icon" aria-hidden="true">
      <AppIcon v-if="tone === 'danger'">
        <circle cx="8" cy="8" r="5.8" />
        <path d="M8 5v3.6" />
        <path d="M8 10.8h.01" />
      </AppIcon>
      <AppIcon v-else-if="tone === 'warning'">
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

    <span class="banner__copy">
      <span class="banner__title">{{ title }}</span>
      <span v-if="detail" class="banner__detail">{{ detail }}</span>
      <!-- Supporting material below the line, e.g. a collapsed server response. -->
      <slot />
    </span>

    <span v-if="$slots.actions" class="banner__actions">
      <slot name="actions" />
    </span>
  </div>
</template>

<style scoped>
.banner {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding: var(--space-sm) var(--space-md);
  border-bottom: var(--border-muted);
  font-size: var(--font-size-sm);
  color: var(--text-primary);
}

.banner__icon {
  display: grid;
  place-items: center;
  flex-shrink: 0;
  width: 18px;
  height: 18px;
}

.banner__icon svg {
  width: 15px;
  height: 15px;
}

.banner--danger {
  background: color-mix(in srgb, var(--accent-red) 10%, var(--bg-surface));
  border-bottom-color: color-mix(in srgb, var(--accent-red) 26%, transparent);
}

.banner--danger .banner__icon {
  color: var(--accent-red);
}

.banner--warning {
  background: color-mix(in srgb, var(--accent-amber) 10%, var(--bg-surface));
  border-bottom-color: color-mix(in srgb, var(--accent-amber) 26%, transparent);
}

.banner--warning .banner__icon {
  color: var(--accent-amber);
}

.banner--info {
  background: color-mix(in srgb, var(--accent-primary) 10%, var(--bg-surface));
  border-bottom-color: color-mix(in srgb, var(--accent-primary) 26%, transparent);
}

.banner--info .banner__icon {
  color: var(--accent-primary);
}

.banner__copy {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: var(--space-2xs) var(--space-sm);
  flex: 1;
  min-width: 0;
}

.banner__title {
  font-weight: var(--weight-semibold);
}

.banner__detail {
  min-width: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.banner__actions {
  display: flex;
  align-items: center;
  gap: var(--space-2xs);
  flex-shrink: 0;
}

/* The card: the copy takes the whole width and wraps; the actions follow on
   their own row, start-aligned, and wrap when the column is narrow. */
.banner--card {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: start;
  row-gap: var(--space-sm);
  padding: var(--space-sm) var(--space-md) var(--space-md) var(--space-sm);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
}

.banner--card.banner--danger { border-color: color-mix(in srgb, var(--accent-red) 26%, transparent); }
.banner--card.banner--warning { border-color: color-mix(in srgb, var(--accent-amber) 26%, transparent); }
.banner--card.banner--info { border-color: color-mix(in srgb, var(--accent-primary) 26%, transparent); }

.banner--card .banner__icon {
  height: calc(var(--font-size-sm) * 1.4);
}

.banner--card .banner__copy {
  flex-direction: column;
  align-items: stretch;
  gap: var(--space-2xs);
}

.banner--card .banner__detail {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 4;
  white-space: normal;
  overflow-wrap: anywhere;
}

.banner--card .banner__actions {
  grid-column: 2;
  flex-wrap: wrap;
  flex-shrink: 1;
  min-width: 0;
}
</style>
