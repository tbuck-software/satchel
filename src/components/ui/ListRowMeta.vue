<script setup lang="ts">
defineProps<{ time: string }>();
</script>

<template>
  <div class="list-row-meta">
    <span class="list-row-meta__time">{{ time }}</span>
    <div class="list-row-meta__actions"><slot /></div>
  </div>
</template>

<style scoped>
.list-row-meta {
  width: calc(2 * var(--space-2xl) + var(--space-xs));
  height: var(--space-5);
  pointer-events: none;
}

.list-row-meta__time {
  display: block;
  padding-top: var(--space-2xs);
  /* When the mail came is information, not a hint: secondary, like the
     reader's own timestamp. */
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  line-height: 1.4;
  font-variant-numeric: tabular-nums;
  text-align: right;
  white-space: nowrap;
  opacity: calc(1 - var(--row-actions-visible, 0));
  transition: opacity var(--duration-fast) var(--ease-out);
}

.list-row-meta__actions {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  opacity: var(--row-actions-visible, 0);
  pointer-events: var(--row-actions-events, none);
  transition: opacity var(--duration-fast) var(--ease-out);
}
</style>
