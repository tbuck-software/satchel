<script setup lang="ts">
withDefaults(
  defineProps<{
    title: string;
    subtitle?: string;
    count?: number;
  }>(),
  {
    subtitle: undefined,
    count: undefined,
  },
);
</script>

<template>
  <header class="pane-header" data-tauri-drag-region>
    <!-- A back arrow, or on the phone the mailbox switcher. The way between
         areas is the tab bar, so a header no longer carries a menu of its own. -->
    <div v-if="$slots.leading" class="pane-header__leading">
      <slot name="leading" />
    </div>
    <div class="pane-header__copy">
      <div class="pane-header__title-row">
        <h2 class="pane-header__title">{{ title }}</h2>
        <span v-if="count !== undefined" class="pane-header__count">{{ count }}</span>
      </div>
      <p v-if="subtitle" class="pane-header__subtitle">{{ subtitle }}</p>
    </div>
    <div v-if="$slots.actions" class="pane-header__actions">
      <slot name="actions" />
    </div>
  </header>
</template>

<style scoped>
.pane-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-sm);
  min-height: 52px;
  padding: var(--space-3-5) var(--space-lg) var(--space-2-5);
  -webkit-app-region: drag;
}

.pane-header__leading {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  flex-shrink: 0;
  -webkit-app-region: no-drag;
}

.pane-header__copy {
  min-width: 0;
  flex: 1;
}

.pane-header__title-row {
  display: flex;
  align-items: baseline;
  gap: var(--space-sm);
  min-width: 0;
}

.pane-header__title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  letter-spacing: -0.01em;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pane-header__count {
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
  color: var(--text-tertiary);
  flex-shrink: 0;
}

.pane-header__subtitle {
  margin: var(--space-2xs) 0 0;
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pane-header__actions {
  display: flex;
  align-items: center;
  gap: var(--space-1-5);
  flex-shrink: 0;
  -webkit-app-region: no-drag;
}
</style>
