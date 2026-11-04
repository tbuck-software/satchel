<script setup lang="ts">
import { defineAsyncComponent, ref, watch } from 'vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import { useUiStore } from '@/stores/ui';
import SidebarMailNav from './SidebarMailNav.vue';

const FolderBrowserPanel = defineAsyncComponent(() => import('./FolderBrowserPanel.vue'));

/**
 * What the drawer offered for mail, on the phone: the lists, the folders and
 * the accounts, in a sheet opened from the mail list's header.
 *
 * The rows are the desktop sidebar's own (`SidebarMailNav`), so a list added
 * there arrives here too. "All folders" swaps the sheet's content for the
 * folder browser rather than stacking a second surface on the first.
 *
 * A view row closes the sheet; an account row does not. The account is the
 * list's scope, and the list behind the sheet switches while it stays open.
 */
const ui = useUiStore();
const foldersOpen = ref(false);

function close() {
  ui.closeMailboxSwitcher();
}

watch(() => ui.mailboxSwitcherOpen, (open) => {
  if (!open) foldersOpen.value = false;
});
</script>

<template>
  <BottomSheet :label="$t('nav.mailboxes.title')" :open="ui.mailboxSwitcherOpen" @dismiss="close">
    <div class="mailbox-sheet">
      <FolderBrowserPanel
        v-if="foldersOpen"
        inline
        :open="foldersOpen"
        @close="foldersOpen = false"
        @navigate="close"
      />
      <template v-else>
        <h2 class="mailbox-sheet__title">{{ $t('nav.mailboxes.title') }}</h2>
        <SidebarMailNav @navigate="close" @open-folders-panel="foldersOpen = true" />
      </template>
    </div>
  </BottomSheet>
</template>

<style scoped>
.mailbox-sheet {
  display: grid;
  padding-bottom: var(--space-md);
}

.mailbox-sheet__title {
  margin: 0;
  padding: var(--space-xs) var(--space-lg) var(--space-sm);
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
}

/* The sidebar lifts the current row off a tinted pane; on the sheet's own
   surface that lift would vanish, so the current list takes the selection tint. */
.mailbox-sheet :deep(.nav-item--active),
.mailbox-sheet :deep(.nav-item--active:hover) {
  background: var(--bg-active);
  box-shadow: none;
}
</style>
