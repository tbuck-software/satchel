<script setup lang="ts">
import AppButton from './AppButton.vue';
import AppIcon from './AppIcon.vue';
import type { ShortcutHint } from '@/types/shortcuts';

withDefaults(defineProps<{
  secondaryLabel: string;
  expanded: boolean;
  popup?: 'menu' | 'dialog';
  controls?: string;
  disabled?: boolean;
  title?: string;
  descriptionId?: string;
  shortcut?: ShortcutHint | null;
}>(), { popup: 'menu', disabled: false });

const emit = defineEmits<{
  click: [event: MouseEvent];
  open: [];
}>();
</script>

<template>
  <span class="split-button">
    <AppButton
      variant="primary"
      class="split-button__primary"
      :disabled="disabled"
      :title="title"
      :aria-describedby="descriptionId"
      :shortcut="shortcut"
      @click="emit('click', $event)"
    >
      <template v-if="$slots.icon" #icon><slot name="icon" /></template>
      <slot />
    </AppButton>
    <AppButton
      variant="primary"
      class="split-button__secondary"
      :disabled="disabled"
      :title="disabled ? title ?? secondaryLabel : secondaryLabel"
      :aria-label="secondaryLabel"
      :aria-haspopup="popup"
      :aria-expanded="expanded"
      :aria-controls="controls"
      :aria-describedby="descriptionId"
      @click="emit('open')"
      @keydown.down.prevent="!disabled && emit('open')"
    >
      <AppIcon><path d="m4.5 6.5 3.5 3.5 3.5-3.5" /></AppIcon>
    </AppButton>
  </span>
</template>

<style scoped>
.split-button {
  display: inline-flex;
  flex: none;
}

.split-button__primary {
  border-start-end-radius: 0;
  border-end-end-radius: 0;
}

.split-button__secondary {
  padding-inline: var(--space-sm);
  border-start-start-radius: 0;
  border-end-start-radius: 0;
  border-inline-start: 1px solid color-mix(in srgb, var(--text-on-accent) 35%, transparent);
}

/* Keep each segment's focus ring above its neighbour. */
.split-button :deep(button:focus-visible) {
  position: relative;
  z-index: 1;
}
</style>
