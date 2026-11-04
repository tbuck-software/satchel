<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useUiStore } from '@/stores/ui';

/**
 * The way back to the rest of the app from a screen that has no pane header.
 *
 * Views built around their own toolbar — calendar, contacts, search — had no
 * drawer handle at all on a phone. Reaching one meant knowing the edge gesture
 * existed, which is the definition of a screen you can get stuck on.
 *
 * `PaneHeader` renders the same control itself; this is for the headers that
 * are not one.
 */
const ui = useUiStore();
</script>

<template>
  <IconButton v-if="ui.isMobile" class="drawer-button" :title="$t('common.menu')" @click="ui.toggleMobileSidebar()">
    <AppIcon>
      <path d="M2 4h12M2 8h12M2 12h12" />
    </AppIcon>
    <span v-if="ui.drawerAttention" class="drawer-button__dot" aria-hidden="true" />
  </IconButton>
</template>

<style scoped>
/* Same dot as PaneHeader's handle. */
.drawer-button {
  position: relative;
}

.drawer-button__dot {
  position: absolute;
  top: 5px;
  right: 5px;
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--accent-orange);
  box-shadow: 0 0 0 2px var(--bg-base);
}
</style>
