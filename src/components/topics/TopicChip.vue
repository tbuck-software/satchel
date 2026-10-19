<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useI18n } from 'vue-i18n';

const { t } = useI18n();

withDefaults(
  defineProps<{
    label: string;
    color?: string | null;
    removable?: boolean;
  }>(),
  {
    color: null,
    removable: false,
  },
);

defineEmits<{
  click: [];
  remove: [];
}>();
</script>

<template>
  <span class="topic-chip" :style="color ? { '--chip-color': color } : undefined">
    <AppButton
      class="topic-chip__label"
      :title="t('topics.chip.open', { name: label })"
      @click="$emit('click')"
      variant="link"
      size="sm"
    >
      {{ label }}
    </AppButton>
    <IconButton
      v-if="removable"
      class="topic-chip__remove"
      :title="t('topics.chip.remove', { name: label })"
      :aria-label="t('topics.chip.remove', { name: label })"
      @click.stop="$emit('remove')"
    >
      ×
    </IconButton>
  </span>
</template>

<style scoped>
.topic-chip {
  --chip-color: var(--accent-primary);
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 1px 4px 1px 8px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--chip-color) 12%, transparent);
  color: var(--chip-color);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-medium);
  max-width: 180px;
}

.topic-chip__label {
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  padding: 0;
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.topic-chip__remove {
  border: 0;
  background: transparent;
  color: inherit;
  font-size: var(--font-size-sm);
  line-height: 1;
  padding: 0 3px;
  border-radius: var(--radius-full);
  cursor: pointer;
  opacity: 0.7;
}

.topic-chip__remove:hover {
  opacity: 1;
  background: color-mix(in srgb, var(--chip-color) 18%, transparent);
}
</style>
