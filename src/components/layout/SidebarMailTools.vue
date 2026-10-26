<script setup lang="ts">
import { defineAsyncComponent, nextTick, ref } from 'vue';
import { useRoute } from 'vue-router';
import { useAccountsStore } from '@/stores/accounts';
import { useUiStore } from '@/stores/ui';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import SearchField from '@/components/ui/SearchField.vue';
import type { AnchorBox } from '@/utils/popoverPlacement';

defineEmits<{ compose: [event: MouseEvent] }>();

// The account filter menu mounts on first open and then stays, so closing
// still plays its transition; until then the sidebar does not load it.
const ContextMenu = defineAsyncComponent(() => import('@/components/ui/ContextMenu.vue'));
const filterMenuMounted = ref(false);

const accounts = useAccountsStore();
const route = useRoute();
const ui = useUiStore();
const filterAnchor = ref<AnchorBox | null>(null);
let filterTrigger: HTMLElement | null = null;

function openFilter(event: MouseEvent) {
  filterTrigger = event.currentTarget as HTMLElement;
  const { left, bottom } = filterTrigger.getBoundingClientRect();
  // A point below the trigger opens the menu beneath the toolbar.
  filterAnchor.value = { left, right: left, top: bottom + 6, bottom: bottom + 6 };
  filterMenuMounted.value = true;
}

function closeFilter() {
  filterAnchor.value = null;
  void nextTick(() => filterTrigger?.focus({ preventScroll: true }));
}

function selectAccount(id: string) {
  accounts.setActiveAccount(id);
  closeFilter();
  ui.closeMobileSidebar();
}
</script>

<template>
  <div class="sidebar-mail-tools">
    <SearchField :active="route.name === 'search'" :label="$t('inbox.searchPlaceholder')" :title="$t('inbox.searchTitle')" @click="ui.openOmni()" />
    <IconButton
      size="lg"
      class="sidebar-mail-tools__filter"
      :title="$t('nav.mailboxFilter')"
      :active="!!accounts.activeAccountId"
      aria-haspopup="menu"
      :aria-expanded="!!filterAnchor"
      @click="openFilter"
    >
      <AppIcon><path d="M2.5 4.5h11M4.5 8h7M6 11.5h4" /></AppIcon>
    </IconButton>
    <IconButton
      size="lg"
      class="sidebar-mail-tools__compose"
      :title="$t('nav.composeNewWindow')"
      :aria-label="$t('nav.composeMessage')"
      @click="$emit('compose', $event)"
    >
      <AppIcon><path d="M8 3v10M3 8h10" /></AppIcon>
    </IconButton>
    <ContextMenu v-if="filterMenuMounted" :open="!!filterAnchor" :anchor="filterAnchor" :label="$t('nav.mailboxFilter')" @close="closeFilter">
      <MenuItem :label="$t('folders.allAccounts')" icon="inbox" :checked="!accounts.activeAccountId" @select="selectAccount('')" />
      <MenuItem
        v-for="account in accounts.accounts"
        :key="account.id"
        :label="account.displayName"
        :title="account.email"
        :checked="accounts.activeAccountId === account.id"
        @select="selectAccount(account.id)"
      >
        <template #icon><span class="account-marker" :style="{ background: account.color }" /></template>
      </MenuItem>
    </ContextMenu>
  </div>
</template>

<style scoped>
.sidebar-mail-tools {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  gap: var(--space-1-5);
  padding: 0 var(--space-md) var(--space-5);
}

.sidebar-mail-tools__filter,
.sidebar-mail-tools__compose {
  border-radius: var(--radius-md);
}

.sidebar-mail-tools__filter:not(.icon-button--active) {
  background: var(--bg-raised);
}

.sidebar-mail-tools .sidebar-mail-tools__compose {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.sidebar-mail-tools .sidebar-mail-tools__compose:hover {
  background: var(--accent-primary-hover);
  color: var(--text-on-accent);
}

.account-marker {
  width: var(--space-2-5);
  height: var(--space-2-5);
  border-radius: var(--radius-full);
}
</style>
