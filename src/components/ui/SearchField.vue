<script setup lang="ts">
/*
 * A search entry that looks like a field and is a button.
 *
 * The sidebar and mobile list share the omni overlay, where operator syntax,
 * scopes and the keyboard route all live. A real <input> here
 * would be a second search box that either duplicates that or, worse, looks
 * like it accepts a query and does something weaker with it.
 */
defineProps<{
  label: string;
  title?: string;
  active?: boolean;
}>();

defineEmits<{
  click: [event: MouseEvent];
}>();
</script>

<template>
  <button type="button" class="search-field" :class="{ 'search-field--active': active }" :aria-current="active ? 'page' : undefined" :title="title" @click="$emit('click', $event)">
    <svg
      class="search-field__icon"
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      aria-hidden="true"
    >
      <circle cx="7" cy="7" r="5" />
      <path d="M11 11L14 14" />
    </svg>
    <span class="search-field__label">{{ label }}</span>
  </button>
</template>

<style scoped>
.search-field {
  display: flex;
  align-items: center;
  gap: var(--space-2-5);
  width: 100%;
  height: 38px;
  padding: 0 var(--space-md);
  border: 0;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  color: var(--text-tertiary);
  font: inherit;
  font-size: var(--font-size-md);
  text-align: left;
  cursor: text;
  transition: background var(--duration-fast) var(--ease-out);
}

.search-field:hover {
  background: var(--bg-hover);
}

/* "You are in search" is a place in the sidebar, marked like the active nav
   item: a raised sheet with petrol ink. The open-row ring would read as a
   focus ring here, beside the query field that actually has focus. */
.search-field--active {
  background: var(--bg-surface);
  color: var(--text-accent);
  box-shadow: var(--shadow-sm);
}

.search-field__icon {
  flex-shrink: 0;
}

.search-field__label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
