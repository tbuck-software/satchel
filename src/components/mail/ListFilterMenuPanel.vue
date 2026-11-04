<script setup lang="ts">
/*
 * The filter menu's content: which account the list is drawn from and how it
 * is laid out. Its own lazily loaded component, so the list header that opens
 * it does not carry the menu in the entry chunk.
 */
import ContextMenu from '@/components/ui/ContextMenu.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import { useAccountsStore } from '@/stores/accounts';
import { useSettingsStore, type InboxGroupingMode } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import type { AnchorBox } from '@/utils/popoverPlacement';

const props = defineProps<{
  anchor: AnchorBox | null;
  showAccounts: boolean;
  showLayout: boolean;
}>();
const emit = defineEmits<{ close: [] }>();

const accounts = useAccountsStore();
const settings = useSettingsStore();
const threads = useThreadsStore();

function selectAccount(id: string) {
  accounts.setActiveAccount(id);
  emit('close');
}

function chooseGroupingMode(mode: InboxGroupingMode) {
  settings.setInboxGroupingMode(mode);
  if (mode === 'classic') threads.closeSmartBucket();
  if (mode === 'smart') threads.categoryFilter = 'all';
  emit('close');
}
</script>

<template>
    <ContextMenu :open="!!props.anchor" :anchor="props.anchor" :label="$t('inbox.filterMenu')" @close="emit('close')">
      <div v-if="props.showAccounts" class="menu-group" role="group" :aria-label="$t('inbox.filterAccounts')">
        <p class="list-filter-menu__label" aria-hidden="true">{{ $t('inbox.filterAccounts') }}</p>
        <MenuItem :label="$t('folders.allAccounts')" icon="inbox" :checked="!accounts.activeAccountId" @select="selectAccount('')" />
        <MenuItem
          v-for="account in accounts.accounts"
          :key="account.id"
          :label="account.displayName"
          :title="account.email"
          :checked="accounts.activeAccountId === account.id"
          @select="selectAccount(account.id)"
        >
          <template #icon><span class="list-filter-menu__account" :style="{ background: account.color }" /></template>
        </MenuItem>
      </div>
      <div v-if="props.showLayout" class="menu-group" role="group" :aria-label="$t('inbox.view')">
        <p class="list-filter-menu__label" aria-hidden="true">{{ $t('inbox.view') }}</p>
        <MenuItem
          :label="$t('inbox.viewGrouped')"
          selection="radio"
          :checked="settings.inboxGroupingMode === 'smart'"
          @select="chooseGroupingMode('smart')"
        />
        <MenuItem
          :label="$t('inbox.viewList')"
          selection="radio"
          :checked="settings.inboxGroupingMode === 'classic'"
          @select="chooseGroupingMode('classic')"
        />
      </div>
    </ContextMenu>
</template>

<style scoped>
.list-filter-menu__label {
  margin: var(--space-2xs) var(--space-2-5);
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.list-filter-menu__account {
  width: var(--space-2-5);
  height: var(--space-2-5);
  border-radius: var(--radius-full);
}
</style>
