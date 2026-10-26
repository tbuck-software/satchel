<script setup lang="ts">
/**
 * One key as the keyboard prints it: `⌘K`, `Ctrl+K`, `g`, `Esc`.
 *
 * The palette and the shortcut help had a keycap each — one quiet grey, one
 * teal monospace — so the same `d` read as two different things one keypress
 * apart. A sequence (`g i`) or a set of alternatives is several caps side by
 * side; the caller lays them out, this draws one.
 */
</script>

<template>
  <kbd class="key-cap"><slot /></kbd>
</template>

<style scoped>
.key-cap {
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  min-width: 20px;
  height: 20px;
  padding: 0 var(--space-1-5);
  border: 1px solid var(--border-color);
  border-bottom-width: 2px;
  border-radius: var(--radius-xs);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-family: inherit;
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
  line-height: 1;
  white-space: nowrap;
}
</style>
