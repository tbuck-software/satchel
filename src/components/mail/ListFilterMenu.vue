<script setup lang="ts">
/*
 * The one filter control of a thread list: which account it is drawn from and
 * how it is laid out. The account choice used to sit in the sidebar and the
 * layout behind a second funnel beside the title — two funnels for one list.
 */
import { computed, defineAsyncComponent, nextTick, ref } from 'vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import { useAccountsStore } from '@/stores/accounts';
import { useSettingsStore, type InboxGroupingMode } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import type { AnchorBox } from '@/utils/popoverPlacement';

const props = defineProps<{
  /** Folders have one layout; the grouped view is a mailbox feature. */
  showLayout: boolean;
}>();

// Mounted on first open and then kept, so closing still plays its transition.
const ContextMenu = defineAsyncComponent(() => import('@/components/ui/ContextMenu.vue'));
const menuMounted = ref(false);

const accounts = useAccountsStore();
const settings = useSettingsStore();
const threads = useThreadsStore();

const anchor = ref<AnchorBox | null>(null);
let trigger: HTMLElement | null = null;

// Even with one account: "All accounts" is how an account scope is left.
const showAccounts = computed(() => accounts.accounts.length > 0);
const visible = computed(() => showAccounts.value || props.showLayout);

function openMenu(event: MouseEvent) {
  trigger = event.currentTarget as HTMLElement;
  const { right, bottom } = trigger.getBoundingClientRect();
  // Right-aligned under the trigger: it sits at the pane's trailing edge.
  anchor.value = { left: right - 232, right: right - 232, top: bottom + 6, bottom: bottom + 6 };
  menuMounted.value = true;
}

function closeMenu() {
  anchor.value = null;
  void nextTick(() => trigger?.focus({ preventScroll: true }));
}

function selectAccount(id: string) {
  accounts.setActiveAccount(id);
  closeMenu();
}

function chooseGroupingMode(mode: InboxGroupingMode) {
  settings.setInboxGroupingMode(mode);
  if (mode === 'classic') threads.closeSmartBucket();
  if (mode === 'smart') threads.categoryFilter = 'all';
  closeMenu();
}
</script>

<template>
  <template v-if="visible">
    <IconButton
      class="list-filter-menu__trigger"
      :title="$t('inbox.filterMenu')"
      :active="!!accounts.activeAccountId || !!anchor"
      aria-haspopup="menu"
      :aria-expanded="!!anchor"
      @click="openMenu"
    >
      <AppIcon><path d="M2.5 4.5h11M4.5 8h7M6 11.5h4" /></AppIcon>
    </IconButton>
    <ContextMenu v-if="menuMounted" :open="!!anchor" :anchor="anchor" :label="$t('inbox.filterMenu')" @close="closeMenu">
      <div v-if="showAccounts" class="menu-group" role="group" :aria-label="$t('inbox.filterAccounts')">
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
</template>

<style scoped>
.list-filter-menu__trigger {
  flex-shrink: 0;
  width: var(--input-height);
  height: var(--input-height);
  border-radius: var(--radius-sm);
}

.list-filter-menu__trigger:not(.icon-button--active) {
  background: var(--bg-raised);
}

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
