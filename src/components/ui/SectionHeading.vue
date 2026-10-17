<script setup lang="ts">
withDefaults(
  defineProps<{
    label: string;
    count?: number;
  }>(),
  {
    count: undefined,
  },
);
</script>

<template>
  <div class="section-heading">
    <h3 class="section-heading__label">{{ label }}</h3>
    <span v-if="count !== undefined" class="section-heading__count">{{ count }}</span>
    <span v-if="$slots.trailing" class="section-heading__trailing">
      <slot name="trailing" />
    </span>
  </div>
</template>

<style scoped>
.section-heading {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  min-height: 24px;
}

.section-heading__label {
  margin: 0;
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  letter-spacing: 0.01em;
}

.section-heading__count {
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
  font-variant-numeric: tabular-nums;
  color: var(--text-tertiary);
}

.section-heading__trailing {
  display: inline-flex;
  align-items: center;
  margin-left: auto;
}
</style>
