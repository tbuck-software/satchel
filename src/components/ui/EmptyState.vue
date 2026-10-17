<script setup lang="ts">
import AppButton from './AppButton.vue';

withDefaults(
  defineProps<{
    icon?: string;
    title: string;
    description?: string;
    actionLabel?: string;
  }>(),
  { icon: '✨', description: '' },
);
const emit = defineEmits<{
  (e: 'action'): void;
}>();
</script>

<template>
  <div class="empty">
    <div class="icon"><slot name="icon">{{ icon }}</slot></div>
    <h3>{{ title }}</h3>
    <p v-if="description">{{ description }}</p>
    <AppButton v-if="actionLabel" variant="primary" size="sm" class="action" @click="emit('action')">
      {{ actionLabel }}
    </AppButton>
  </div>
</template>

<style scoped>
.empty {
  display: grid;
  gap: var(--space-sm);
  justify-items: center;
  text-align: center;
  padding: calc(var(--space-2xl) * 2) var(--space-5);
  color: var(--text-secondary);
}
.icon {
  font-size: 48px;
  line-height: 1;
  margin-bottom: var(--space-xs);
}
h3 {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  font-weight: var(--weight-semibold);
}
p {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  max-width: 360px;
}
/* The kit's own button, so this one cannot drift from the six it replaced. */
.action {
  margin-top: var(--space-xs);
}
</style>
