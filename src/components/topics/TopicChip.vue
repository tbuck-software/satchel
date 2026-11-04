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
  /* The topic's colour leans toward the ink: pure blue on its own pale tint
     was the weakest text in the header. */
  color: color-mix(in srgb, var(--chip-color) 55%, var(--text-primary));
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

/*
  Touch: the pill stays 30px, and two targets share it without a gap. The
  name's button runs from the pill's left edge to the ×, so the whole chip
  opens the topic (AppButton reaches its hit area to 44px above and below);
  the × gets a 44px square of its own, its height reaching past the pill the
  same way, instead of growing the pill to 44px and the header with it.
*/
@media (pointer: coarse) {
  .topic-chip {
    height: 30px;
    padding: 0;
  }

  .topic-chip__label {
    align-self: stretch;
    padding-inline: var(--space-md) var(--space-2xs);
  }

  .topic-chip .topic-chip__remove {
    position: relative;
    align-self: stretch;
    min-height: 0;
    min-width: var(--touch-target);
    padding: 0;
  }

  .topic-chip .topic-chip__remove::after {
    content: '';
    position: absolute;
    inset: -7px 0;
  }
}
</style>
