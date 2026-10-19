<script setup lang="ts">
import AppButton from './AppButton.vue';
import AppIcon from './AppIcon.vue';

/*
 * The picture is decoration: the title says what is empty. It used to be a
 * colour emoji passed as a string, which screen readers read out ("sparkles",
 * "open mailbox with lowered flag") and which ignored the theme. Pass an
 * `AppIcon` glyph through the `icon` slot instead.
 */
withDefaults(
  defineProps<{
    title: string;
    description?: string;
    actionLabel?: string;
  }>(),
  { description: '' },
);
const emit = defineEmits<{
  (e: 'action'): void;
}>();
</script>

<template>
  <div class="empty">
    <div class="icon" aria-hidden="true">
      <slot name="icon">
        <AppIcon size="lg">
          <path d="M2.5 9.5 4.3 3.8A1 1 0 0 1 5.3 3h5.4a1 1 0 0 1 1 .8l1.8 5.7" />
          <path d="M2.5 9.5V12a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1V9.5h-3.2l-.8 1.5h-3l-.8-1.5H2.5Z" />
        </AppIcon>
      </slot>
    </div>
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
  display: grid;
  place-items: center;
  margin-bottom: var(--space-xs);
}
/* A glyph from the kit sits on a round tile; drawn illustrations stay as they are. */
.icon :deep(.app-icon) {
  box-sizing: content-box;
  padding: var(--space-3-5);
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  color: var(--text-tertiary);
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
