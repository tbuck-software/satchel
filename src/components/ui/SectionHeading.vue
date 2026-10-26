<script setup lang="ts">
withDefaults(
  defineProps<{
    label: string;
    count?: number;
    /** What the section is about, after the label: quieter and truncated. */
    detail?: string;
  }>(),
  {
    count: undefined,
    detail: undefined,
  },
);
</script>

<template>
  <div class="section-heading">
    <h3 class="section-heading__label" :class="{ 'section-heading__label--detailed': detail }">
      {{ label }}<span v-if="detail" class="section-heading__detail"> · {{ detail }}</span>
    </h3>
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

/* A detail is open-ended (a subject line), so it is the part that gives way. */
.section-heading__label--detailed {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.section-heading__detail {
  font-weight: var(--weight-regular);
  color: var(--text-tertiary);
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
