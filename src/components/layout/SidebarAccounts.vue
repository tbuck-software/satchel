<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useThreadsStore } from '@/stores/threads';
import { useUiStore } from '@/stores/ui';
import NavItem from '@/components/ui/NavItem.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import AccountAvatar from '@/components/common/AccountAvatar.vue';
import type { AnchorBox } from '@/utils/popoverPlacement';

const SidebarAccountMenu = defineAsyncComponent(() => import('./SidebarAccountMenu.vue'));

const { t } = useI18n();
const accounts = useAccountsStore();
const mailboxes = useMailboxesStore();
const threads = useThreadsStore();
const router = useRouter();
const ui = useUiStore();
function open(accountId: string) {
  accounts.setActiveAccount(accountId);
  threads.workflowFilter = 'all';
  threads.setMailboxFilter({ kind: 'section', value: 'inbox' });
  ui.closeMobileSidebar();
  void router.push({ name: 'inbox' });
}

// A lone account that is always visible has nothing to switch to or look
// in on; the section only appears once there is a choice or a schedule.
const shown = computed(() => accounts.emailAccounts.length > 1
  || accounts.emailAccounts.some((account) => accounts.visibilityFor(account.id)?.policy.mode !== undefined
    && accounts.visibilityFor(account.id)?.policy.mode !== 'always'));

/**
 * The Inbox's own number, per account: unread threads in the Inbox that are
 * not set aside, done or snoozed. Without an account it is the sum over the
 * visible ones — exactly the badge on Inbox below.
 */
function inboxCount(accountId?: string): number {
  return mailboxes.countForMailboxType('inbox', accountId);
}

// The accounts "All accounts" stands for: the visible ones, at most three.
const stack = computed(() => {
  const visible = accounts.emailAccounts.filter((account) => accounts.isVisible(account.id));
  return (visible.length ? visible : accounts.emailAccounts).slice(0, 3);
});

const menuAccountId = ref('');
const menuAnchor = ref<AnchorBox | null>(null);
function openMenu(accountId: string, event: MouseEvent) {
  // The context-menu key fires at the focused row, not at a pointer.
  const box = event.clientX || event.clientY
    ? { left: event.clientX, top: event.clientY, right: event.clientX, bottom: event.clientY }
    : (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuAccountId.value = accountId;
  menuAnchor.value = { left: box.left, top: box.top, right: box.right, bottom: box.bottom };
}
</script>

<template>
  <nav v-if="shown" class="sidebar-accounts" :aria-label="t('accountVisibility.accounts')">
    <SectionHeading :label="t('accountVisibility.accounts')" />
    <NavItem v-if="accounts.emailAccounts.length > 1" :label="t('folders.allAccounts')" :active="!accounts.activeAccountId" :count="inboxCount()" @click="open('')">
      <template #leading>
        <span class="sidebar-accounts__stack" :class="`sidebar-accounts__stack--${stack.length}`" aria-hidden="true">
          <AccountAvatar v-for="account in stack" :key="account.id" :name="account.displayName" :color="account.color" :avatar="account.avatar" :size="11" />
        </span>
      </template>
    </NavItem>
    <div
      v-for="account in accounts.emailAccounts"
      :key="account.id"
      class="sidebar-accounts__row"
      :class="{ 'sidebar-accounts__row--resting': !accounts.isVisible(account.id) }"
      @contextmenu.prevent="openMenu(account.id, $event)"
    >
      <NavItem :label="account.displayName" :active="accounts.activeAccountId === account.id" :count="inboxCount(account.id)" :title="accounts.isVisible(account.id) ? t('accountVisibility.openAccount', { account: account.email }) : t('accountVisibility.restingHint', { account: account.email })" @click="open(account.id)">
        <template #leading>
          <AccountAvatar :name="account.displayName" :color="account.color" :avatar="account.avatar" :size="18" :resting="!accounts.isVisible(account.id)" />
        </template>
        <!-- Resting: still listed, but asleep. No number pulls at you; a zZ
             sits where it would be. -->
        <template v-if="!accounts.isVisible(account.id)" #trailing>
          <span class="sidebar-accounts__asleep" role="img" :aria-label="t('accountVisibility.resting')">
            <AppIcon><path d="M7.5 2.5h6l-6 6.5h6" /><path d="M2.5 9h3.5l-3.5 4.5h3.5" /></AppIcon>
          </span>
        </template>
      </NavItem>
    </div>
    <SidebarAccountMenu v-if="menuAccountId" :account-id="menuAccountId" :anchor="menuAnchor" @close="menuAnchor = null" />
  </nav>
</template>

<style scoped>
.sidebar-accounts { display: grid; gap: var(--space-2xs); padding: var(--space-md) var(--space-md) var(--space-xs); }
.sidebar-accounts__row { display: flex; align-items: center; min-width: 0; }
.sidebar-accounts__row > :first-child { flex: 1; }
.sidebar-accounts__row--resting { color: var(--text-tertiary); }
.sidebar-accounts__row--resting :deep(.nav-item) { color: var(--text-tertiary); }
.sidebar-accounts__asleep { display: inline-flex; color: var(--text-tertiary); }

/* Rings cut each avatar out of the one behind it, in the row's own colour. */
.sidebar-accounts { --avatar-ring: var(--bg-sidebar); }
.sidebar-accounts :deep(.nav-item--active) { --avatar-ring: var(--bg-surface); }

/* "All accounts": up to three small faces in the box of one avatar, which
   is the 18px of the icons below, so every label starts on the same line. */
.sidebar-accounts__stack { position: relative; width: 18px; height: 18px; flex-shrink: 0; }
.sidebar-accounts__stack > * { position: absolute; border-radius: var(--radius-full); box-shadow: 0 0 0 1.5px var(--avatar-ring); }
.sidebar-accounts__stack--1 > :nth-child(1) { left: 3px; top: 3px; }
.sidebar-accounts__stack--2 > :nth-child(1) { left: 0; top: 1px; }
.sidebar-accounts__stack--2 > :nth-child(2) { left: 7px; top: 6px; }
.sidebar-accounts__stack--3 > :nth-child(1) { left: 0; top: 0; }
.sidebar-accounts__stack--3 > :nth-child(2) { left: 7px; top: 0; }
.sidebar-accounts__stack--3 > :nth-child(3) { left: 3.5px; top: 7px; }
</style>
