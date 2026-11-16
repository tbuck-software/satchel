<script setup lang="ts">
import { computed } from 'vue';

const props = defineProps<{ value: number; max: number; label: string; valueText?: string }>();
const value = computed(() => Math.min(Math.max(0, props.value), Math.max(0, props.max)));
const percent = computed(() => props.max > 0 ? value.value / props.max * 100 : 0);
</script>

<template>
  <div class="progress-bar" role="progressbar" :aria-label="label" aria-valuemin="0"
    :aria-valuemax="max" :aria-valuenow="value" :aria-valuetext="valueText">
    <span class="progress-bar__fill" :style="{ width: `${percent}%` }" />
  </div>
</template>

<style scoped>
.progress-bar { height: var(--space-xs); overflow: hidden; border-radius: var(--radius-full); background: var(--bg-inset); }
.progress-bar__fill { display: block; height: 100%; border-radius: inherit; background: var(--accent-primary); transition: width var(--duration-normal) var(--ease-out); }
</style>
