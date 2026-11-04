<script setup lang="ts">
/**
 * A row of top-level destinations along the bottom edge, as on iOS and
 * Android: icon over label, the whole column the target.
 *
 * It sits on the bottom safe area and pads itself by it, so its surface runs
 * to the screen edge while the targets stay above the home indicator. The
 * current tab is marked three ways — accent ink, a tinted pill behind a
 * heavier icon, a bolder label — so it does not hang on colour alone.
 *
 * Tapping the tab you are on emits `reselect` rather than `select`: the host
 * decides what "again" means (iOS scrolls back to the top).
 */
export interface TabBarItem {
  id: string;
  label: string;
}

const props = defineProps<{
  /** Names the navigation landmark. */
  label: string;
  items: TabBarItem[];
  /** The current tab's id, or null when none of them is current. */
  active: string | null;
}>();

const emit = defineEmits<{
  select: [id: string];
  reselect: [id: string];
}>();

function onClick(id: string) {
  if (id === props.active) emit('reselect', id);
  else emit('select', id);
}
</script>

<template>
  <nav class="tab-bar" :aria-label="label" :style="{ '--tab-count': items.length }">
    <button
      v-for="item in items"
      :key="item.id"
      type="button"
      class="tab-bar__item"
      :class="{ 'tab-bar__item--active': item.id === active }"
      :aria-current="item.id === active ? 'page' : undefined"
      :data-tab="item.id"
      @click="onClick(item.id)"
    >
      <span class="tab-bar__icon" aria-hidden="true">
        <slot name="icon" :item="item" :active="item.id === active" />
      </span>
      <span class="tab-bar__label">{{ item.label }}</span>
    </button>
  </nav>
</template>

<style scoped>
.tab-bar {
  display: grid;
  flex-shrink: 0;
  grid-template-columns: repeat(var(--tab-count), minmax(0, 1fr));
  padding:
    0
    var(--satchel-safe-area-right)
    var(--satchel-safe-area-bottom)
    var(--satchel-safe-area-left);
  /* A hairline, not a shadow: the list ends here, it does not slide under. */
  border-top: 1px solid var(--border-color-muted);
  background: var(--bg-surface);
}

.tab-bar__item {
  display: grid;
  justify-items: center;
  align-content: center;
  gap: var(--space-2xs);
  min-width: 0;
  min-height: var(--tab-bar-height);
  padding: var(--space-1-5) var(--space-2xs) var(--space-xs);
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-medium);
  cursor: pointer;
  -webkit-tap-highlight-color: transparent;
}

.tab-bar__icon {
  display: grid;
  place-items: center;
  width: calc(2 * var(--space-xl));
  height: calc(var(--space-xl) + var(--space-xs));
  border-radius: var(--radius-full);
  transition: background var(--duration-fast) var(--ease-out);
}

.tab-bar__label {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tab-bar__item--active {
  color: var(--text-accent);
  font-weight: var(--weight-bold);
}

.tab-bar__item--active .tab-bar__icon {
  background: color-mix(in srgb, var(--accent-primary) 16%, transparent);
}

.tab-bar__item--active .tab-bar__icon :deep(svg) {
  stroke-width: 2;
}
</style>
