<script setup lang="ts">
withDefaults(defineProps<{ disabled?: boolean }>(), { disabled: false });
defineEmits<{ click: [event: MouseEvent] }>();
</script>

<template>
  <button type="button" class="list-row-button" :disabled="disabled" @click="$emit('click', $event)">
    <slot />
  </button>
</template>

<style scoped>
.list-row-button {
  width: 100%;
  min-width: 0;
  border: 0;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-primary);
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition: background-color var(--duration-fast) var(--ease-out);
}

.list-row-button:hover:not(:disabled) {
  background: var(--bg-hover);
}

.list-row-button:disabled {
  cursor: default;
}
</style>
