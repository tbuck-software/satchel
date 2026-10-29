<script setup lang="ts">
withDefaults(
  defineProps<{
    label: string;
    active?: boolean;
    /**
     * Set it and the row becomes a toggle: `active` then means pressed, and
     * assistive technology is told so. Leave it out for navigation, where
     * `active` means "you are here" and `aria-pressed` would be a lie.
     */
    pressed?: boolean;
    count?: number;
    countTone?: 'neutral' | 'accent' | 'danger';
    indent?: number;
    title?: string;
  }>(),
  {
    active: false,
    pressed: undefined,
    count: undefined,
    countTone: 'neutral',
    indent: 0,
    title: undefined,
  },
);

defineEmits<{
  click: [event: MouseEvent];
}>();
</script>

<template>
  <button
    type="button"
    class="nav-item"
    :class="{ 'nav-item--active': active }"
    :style="indent > 0 ? { paddingLeft: `${10 + indent * 18}px` } : undefined"
    :aria-pressed="pressed"
    :title="title"
    @click="$emit('click', $event)"
  >
    <span v-if="$slots.icon" class="nav-item__icon">
      <slot name="icon" />
    </span>
    <span v-if="$slots.leading" class="nav-item__leading">
      <slot name="leading" />
    </span>
    <span class="nav-item__label"><slot>{{ label }}</slot></span>
    <span v-if="$slots.trailing" class="nav-item__trailing">
      <slot name="trailing" />
    </span>
    <span v-else-if="count !== undefined && count > 0" class="nav-item__count" :class="`nav-item__count--${countTone}`">
      {{ count }}
    </span>
  </button>
</template>

<style scoped>
/* A phone row is a thumb's height, not a pointer's. */
@media (pointer: coarse) {
  .nav-item {
    min-height: 44px;
  }
}

.nav-item {
  display: flex;
  align-items: center;
  gap: var(--space-2-5);
  width: 100%;
  min-width: 0;
  min-height: 34px;
  padding: var(--space-1-5) var(--space-2-5);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-medium);
  text-align: left;
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.nav-item:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

/* A sheet lifted off the pane, not a saturated pill. The accent is spent on
   one thing per pane — here it is the count, which is what you scan for. */
.nav-item--active,
.nav-item--active:hover {
  background: var(--bg-surface);
  color: var(--text-primary);
  box-shadow: var(--shadow-sm);
  font-weight: var(--weight-semibold);
}

.nav-item__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  flex-shrink: 0;
  color: var(--text-tertiary);
  transition: color var(--duration-fast) var(--ease-out);
}

.nav-item:hover .nav-item__icon {
  color: var(--text-secondary);
}

.nav-item--active .nav-item__icon,
.nav-item--active:hover .nav-item__icon {
  color: var(--text-primary);
}

.nav-item__leading {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}

.nav-item__label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.nav-item__trailing {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}

.nav-item__count {
  flex-shrink: 0;
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
  color: var(--text-tertiary);
}

.nav-item__count--accent {
  color: var(--accent-primary);
  font-weight: var(--weight-semibold);
}

.nav-item__count--danger {
  color: var(--accent-red);
  font-weight: var(--weight-semibold);
}

.nav-item--active .nav-item__count {
  color: var(--text-secondary);
}

/* Same breakpoint that lifts the type scale and splits list from detail: at
   arm's length a 34px row is a miss waiting to happen. Call sites used to
   rebuild the row to get this. */
@media (max-width: 1020px) {
  .nav-item {
    min-height: 48px;
    padding: var(--space-2-5) var(--space-md);
  }
}
</style>
