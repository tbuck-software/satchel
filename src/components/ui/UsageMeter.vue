<script setup lang="ts">
import { computed } from 'vue';

/**
 * How much of something is used: a thin bar with `role="meter"`. For a fixed
 * capacity (a mailbox quota, a storage target), not for progress through a
 * task — that is a `progressbar`, and it moves.
 */
const props = withDefaults(
  defineProps<{
    value: number;
    max: number;
    /** The accessible name: what is being measured. */
    label: string;
    /** What a screen reader says for the value, e.g. "1.2 GB of 2 GB used". */
    valueText?: string;
    tone?: 'neutral' | 'warning' | 'danger';
  }>(),
  { tone: 'neutral', valueText: undefined },
);

const percent = computed(() => {
  if (!(props.max > 0)) return 0;
  return Math.min(100, Math.max(0, (props.value / props.max) * 100));
});
</script>

<template>
  <div
    class="usage-meter"
    :class="`usage-meter--${tone}`"
    role="meter"
    :aria-label="label"
    aria-valuemin="0"
    :aria-valuemax="max"
    :aria-valuenow="Math.min(Math.max(0, value), max)"
    :aria-valuetext="valueText"
  >
    <span class="usage-meter__fill" :style="{ width: `${percent}%` }" />
  </div>
</template>

<style scoped>
.usage-meter {
  overflow: hidden;
  height: 6px;
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
}

.usage-meter__fill {
  display: block;
  height: 100%;
  border-radius: var(--radius-full);
  background: var(--accent-primary);
  transition: width var(--duration-fast) var(--ease-out);
}

.usage-meter--warning .usage-meter__fill { background: var(--accent-amber); }
.usage-meter--danger .usage-meter__fill { background: var(--accent-red); }
</style>
