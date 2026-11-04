<script setup lang="ts">
import AppIcon from './AppIcon.vue';
import CardButton from './CardButton.vue';

/**
 * The phone's large title, as on iOS: the name of the list in display type,
 * and the name itself the control that switches lists ("Inbox ▾"). Actions
 * share the row on the right; the search field follows below, full width.
 */
defineProps<{
  title: string;
  /** Account scope and the like, in small type under the title. */
  subtitle?: string;
  /** Accessible name of the title button; starts with the title it shows. */
  switchLabel: string;
}>();

const emit = defineEmits<{ switch: [] }>();
</script>

<template>
  <header class="large-title">
    <CardButton class="large-title__switch" :aria-label="switchLabel" aria-haspopup="dialog" @click="emit('switch')">
      <span class="large-title__text">{{ title }}</span>
      <AppIcon class="large-title__chevron" size="lg"><path d="m4.5 6.5 3.5 3.5 3.5-3.5" /></AppIcon>
    </CardButton>
    <div v-if="$slots.actions" class="large-title__actions">
      <slot name="actions" />
    </div>
    <p v-if="subtitle" class="large-title__subtitle">{{ subtitle }}</p>
  </header>
</template>

<style scoped>
.large-title {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  column-gap: var(--space-sm);
  padding: var(--space-sm) var(--space-lg) var(--space-2-5);
}

.large-title__switch {
  justify-self: start;
  align-items: center;
  gap: var(--space-xs);
  max-width: 100%;
  min-height: var(--touch-target);
  color: var(--text-primary);
}

.large-title__text {
  min-width: 0;
  overflow: hidden;
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.large-title__chevron {
  color: var(--text-tertiary);
}

.large-title__actions {
  display: flex;
  align-items: center;
  gap: var(--space-1-5);
}

.large-title__subtitle {
  grid-column: 1 / -1;
  margin: calc(-1 * var(--space-xs)) 0 0;
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
