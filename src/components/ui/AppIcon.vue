<script setup lang="ts">
/**
 * The frame every stroke icon is drawn in: one grid, one line weight, three
 * sizes. The glyph is a Lucide icon passed as `icon` (`import { Ban } from
 * 'lucide'`); the slot is left for the few marks Lucide has no word for.
 *
 * Hand-written `<svg>`s had drifted to fourteen stroke widths and eighteen
 * render sizes, so two icons side by side in one menu read as two sets.
 */
import { computed } from 'vue';
import type { IconNode } from 'lucide';

const props = withDefaults(
  defineProps<{
    /** A Lucide icon. Drawn on its 24 grid; `grid` is ignored. */
    icon?: IconNode | null;
    size?: 'sm' | 'md' | 'lg';
    /**
     * The grid a slotted glyph's coordinates were drawn on. Anything but 16
     * is scaled into the 16 viewBox, with the stroke compensated so the line
     * weight stays the same.
     */
    grid?: 16 | 20 | 24;
  }>(),
  { icon: null, size: 'md', grid: 16 },
);

const PIXELS = { sm: 14, md: 16, lg: 20 } as const;

const pixels = computed(() => PIXELS[props.size]);
const scale = computed(() => 16 / (props.icon ? 24 : props.grid));
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
    <g v-if="props.icon" :transform="`scale(${scale})`" :stroke-width="1.5 / scale">
      <component :is="tag" v-for="([tag, attrs], index) in props.icon" :key="index" v-bind="attrs" />
    </g>
    <slot v-else-if="props.grid === 16" />
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
