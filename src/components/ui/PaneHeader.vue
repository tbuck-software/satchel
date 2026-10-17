<script setup lang="ts">
import IconButton from './IconButton.vue';
import { useUiStore } from '@/stores/ui';
import AppIcon from '@/components/ui/AppIcon.vue';

withDefaults(
  defineProps<{
    title: string;
    subtitle?: string;
    count?: number;
    /**
     * Suppress the drawer handle. For headers that are already a step *inside*
     * something — a thread, a topic — where the leading slot carries back and a
     * second way out would only compete with it.
     */
    noMenu?: boolean;
  }>(),
  {
    subtitle: undefined,
    count: undefined,
    noMenu: false,
  },
);

const ui = useUiStore();
</script>

<template>
  <header class="pane-header" data-tauri-drag-region>
    <!-- Every pane header offers the drawer on a phone, so no screen can be a
         place you get into and cannot get out of. An explicit leading slot wins:
         a back arrow is the more specific answer to "how do I leave". -->
    <div v-if="$slots.leading || (ui.isMobile && !noMenu)" class="pane-header__leading">
      <slot v-if="$slots.leading" name="leading" />
      <IconButton v-else :title="$t('common.menu')" @click="ui.toggleMobileSidebar()">
        <AppIcon size="lg">
          <path d="M2 4h12M2 8h12M2 12h12" />
        </AppIcon>
      </IconButton>
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
