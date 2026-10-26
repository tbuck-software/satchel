<script setup lang="ts">
defineProps<{
  options: Array<{ value: string; label: string; disabled?: boolean; title?: string }>;
  modelValue: string;
  ariaLabel?: string;
}>();

defineEmits<{
  'update:modelValue': [value: string];
}>();
</script>

<template>
  <div class="segmented" role="group" :aria-label="ariaLabel">
    <button
      v-for="option in options"
      :key="option.value"
      type="button"
      class="segmented__option"
      :class="{ 'segmented__option--active': option.value === modelValue }"
      :aria-pressed="option.value === modelValue"
      :disabled="option.disabled"
      :title="option.title"
      @click="$emit('update:modelValue', option.value)"
    >
      {{ option.label }}
    </button>
  </div>
</template>

<style scoped>
.segmented {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2xs);
  padding: var(--space-2xs);
  border-radius: var(--radius-full);
  /* A well below whatever it sits on: the settings card is --bg-raised
     itself, so the track needs a recessed fill and a hairline to show. */
  background: var(--bg-inset);
  box-shadow: inset 0 0 0 1px var(--border-color-muted);
}

.segmented__option {
  height: 26px;
  padding: 0 var(--space-2-5);
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.segmented__option:not(.segmented__option--active):hover:not(:disabled) {
  color: var(--text-primary);
}

.segmented__option:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

/* The chosen segment is a raised neutral sheet, like an active nav item —
   the petrol fill stays for primary actions. */
.segmented__option--active {
  background: var(--bg-overlay);
  color: var(--text-primary);
  box-shadow: var(--shadow-sm);
}
</style>
