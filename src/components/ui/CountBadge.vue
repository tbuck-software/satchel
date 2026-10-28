<script setup lang="ts">
import { computed } from 'vue';

const props = withDefaults(
  defineProps<{
    value: number;
    tone?: 'neutral' | 'accent' | 'danger';
    max?: number;
  }>(),
  {
    tone: 'neutral',
    max: 999,
  },
);

const display = computed(() => (props.value > props.max ? `${props.max}+` : `${props.value}`));
</script>

<template>
  <span class="count-badge" :class="`count-badge--${tone}`">{{ display }}</span>
</template>

<style scoped>
.count-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  height: 18px;
  padding: 0 var(--space-1-5);
  border-radius: var(--radius-full);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}

.count-badge--neutral {
  /* The overlay surface with a touch of ink: on paper the overlay alone is the
     same white as a selected row, and the badge vanished into it. */
  background: color-mix(in srgb, var(--text-primary) 9%, var(--bg-overlay));
  color: var(--text-secondary);
}

.count-badge--accent {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.count-badge--danger {
  background: color-mix(in srgb, var(--accent-red) 20%, transparent);
  color: var(--accent-red);
}
</style>
