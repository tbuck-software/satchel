<script setup lang="ts">
/**
 * The frame every stroke icon is drawn in: one grid, one line weight, three
 * sizes. The glyph itself comes through the slot.
 *
 * Hand-written `<svg>`s had drifted to fourteen stroke widths and eighteen
 * render sizes, so two icons side by side in one menu read as two sets.
 */
import { computed } from 'vue';

const props = withDefaults(
  defineProps<{
    size?: 'sm' | 'md' | 'lg';
    /**
     * The grid the glyph's coordinates were drawn on. Anything but 16 is
     * scaled into the 16 viewBox, with the stroke compensated so the line
     * weight stays the same.
     */
    grid?: 16 | 20 | 24;
  }>(),
  { size: 'md', grid: 16 },
);

const PIXELS = { sm: 14, md: 16, lg: 20 } as const;

const pixels = computed(() => PIXELS[props.size]);
const scale = computed(() => 16 / props.grid);
</script>

<template>
  <svg
    class="app-icon"
    :width="pixels"
    :height="pixels"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
  >
    <slot v-if="props.grid === 16" />
    <g v-else :transform="`scale(${scale})`" :stroke-width="1.5 / scale">
      <slot />
    </g>
  </svg>
</template>

<style scoped>
.app-icon {
  flex-shrink: 0;
}
</style>
