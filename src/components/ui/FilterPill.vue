<script setup lang="ts">
/*
 * A pill in a filter strip: pressed or not, optionally with the colour that
 * identifies what it filters by and the number of things behind it.
 *
 * `pressed` rather than `active`: a filter strip is a set of toggles, and
 * assistive technology should hear which one is on, not "you are here".
 */
withDefaults(
  defineProps<{
    label: string;
    pressed?: boolean;
    /** The identity colour of what this filters by — an account, a source. */
    dot?: string | null;
    count?: number;
  }>(),
  {
    pressed: false,
    dot: null,
    count: undefined,
  },
);

defineEmits<{
  click: [event: MouseEvent];
}>();
</script>

<template>
  <button
    type="button"
    class="filter-pill"
    :class="{ 'filter-pill--pressed': pressed }"
    :aria-pressed="pressed"
    @click="$emit('click', $event)"
  >
    <span v-if="dot" class="filter-pill__dot" :style="{ background: dot }" aria-hidden="true" />
    <span class="filter-pill__label">{{ label }}</span>
    <span v-if="count !== undefined && count > 0" class="filter-pill__count">{{ count }}</span>
  </button>
</template>

<style scoped>
.filter-pill {
  display: flex;
  align-items: center;
  gap: var(--space-1-5);
  flex-shrink: 0;
  height: 28px;
  padding: 0 var(--space-2-5);
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-xs);
  font-weight: var(--weight-medium);
  white-space: nowrap;
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.filter-pill:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.filter-pill--pressed,
.filter-pill--pressed:hover {
  background: var(--action-neutral);
  color: var(--action-neutral-ink);
  font-weight: var(--weight-semibold);
}

.filter-pill__dot {
  width: 8px;
  height: 8px;
  flex-shrink: 0;
  border-radius: var(--radius-full);
}

.filter-pill__count {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-variant-numeric: tabular-nums;
}

.filter-pill--pressed .filter-pill__count {
  color: var(--action-neutral-ink);
}

@media (pointer: coarse) {
  .filter-pill {
    height: 36px;
  }
}
</style>
