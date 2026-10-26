<script setup lang="ts">
import { computed, ref } from 'vue';
import AppIcon from './AppIcon.vue';

defineOptions({ inheritAttrs: false });
const props = defineProps<{
  modelValue: string;
  label: string;
  placeholder?: string;
}>();
const emit = defineEmits<{ 'update:modelValue': [value: string] }>();
/*
 * A plain prop/emit pair rather than `defineModel`: Vue's `useModel` helper
 * would otherwise be the only reason it sits in the shared `vendor-vue` chunk
 * every window loads at startup.
 */
const model = computed({
  get: () => props.modelValue,
  set: (value: string) => emit('update:modelValue', value),
});
const input = ref<HTMLInputElement | null>(null);

defineExpose({
  focus: () => input.value?.focus(),
  select: () => input.value?.select(),
});
</script>

<template>
  <div class="search-input-field" :class="{ 'search-input-field--trailing': $slots.trailing }">
    <AppIcon class="search-input-field__icon"><circle cx="7" cy="7" r="4.5" /><path d="m10.5 10.5 3 3" /></AppIcon>
    <input
      v-bind="$attrs"
      ref="input"
      v-model="model"
      class="search-input"
      type="search"
      inputmode="search"
      autocomplete="off"
      :aria-label="label"
      :placeholder="placeholder"
    />
    <span v-if="$slots.trailing" class="search-input-field__trailing"><slot name="trailing" /></span>
  </div>
</template>

<style scoped>
.search-input-field {
  position: relative;
  min-width: 0;
  width: 100%;
}

.search-input {
  width: 100%;
  min-width: 0;
  height: var(--input-height);
  padding: 0 var(--space-md) 0 calc(var(--space-md) + var(--space-lg) + var(--space-sm));
  border: var(--border-default);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-md);
  transition: border-color var(--duration-fast) var(--ease-out);
}

.search-input:focus {
  border-color: var(--accent-primary);
}

.search-input::placeholder {
  color: var(--text-tertiary);
}

.search-input-field--trailing .search-input {
  padding-inline-end: var(--input-height);
}

.search-input-field--trailing .search-input::-webkit-search-cancel-button {
  -webkit-appearance: none;
  display: none;
}

.search-input-field__icon,
.search-input-field__trailing {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  color: var(--text-tertiary);
}

.search-input-field__icon {
  inset-inline-start: var(--space-md);
  pointer-events: none;
}

.search-input-field__trailing {
  display: inline-flex;
  align-items: center;
  inset-inline-end: var(--space-xs);
}
</style>
