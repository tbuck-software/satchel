<script setup lang="ts">
import { computed } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useThreadsStore } from '@/stores/threads';
import { useUiStore } from '@/stores/ui';
import NavItem from '@/components/ui/NavItem.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';

const { t } = useI18n();
const accounts = useAccountsStore();
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
</script>

<template>
  <nav v-if="shown" class="sidebar-accounts" :aria-label="t('accountVisibility.accounts')">
    <SectionHeading :label="t('accountVisibility.accounts')" />
    <NavItem v-if="accounts.emailAccounts.length > 1" :label="t('folders.allAccounts')" :active="!accounts.activeAccountId" @click="open('')" />
    <div v-for="account in accounts.emailAccounts" :key="account.id" class="sidebar-accounts__row" :class="{ 'sidebar-accounts__row--resting': !accounts.isVisible(account.id) }">
      <NavItem :label="account.displayName" :active="accounts.activeAccountId === account.id" :title="t('accountVisibility.openAccount', { account: account.email })" @click="open(account.id)">
        <template #icon>
          <AppIcon v-if="!accounts.isVisible(account.id)"><path d="M13.5 9.5A5.8 5.8 0 0 1 6.5 2 6 6 0 1 0 13.5 9.5Z" /></AppIcon>
          <span v-else class="sidebar-accounts__dot" :style="{ background: account.color }" />
        </template>
        <span class="sidebar-accounts__identity"><span>{{ account.displayName }}</span><span v-if="!accounts.isVisible(account.id)" class="sidebar-accounts__status">{{ t('accountVisibility.resting') }}</span></span>
      </NavItem>
      <AppButton v-if="!accounts.isVisible(account.id)" variant="link" size="sm" :disabled="accounts.visibilityBusy" :title="t('accountVisibility.showAccount', { account: account.displayName })" @click="accounts.showToday(account.id, true)">{{ t('accountVisibility.showToday') }}</AppButton>
      <AppButton v-else-if="accounts.visibilityFor(account.id)?.policy.mode !== 'always' || accounts.visibilityFor(account.id)?.policy.override_date === accounts.visibilityFor(account.id)?.local_date" variant="link" size="sm" :disabled="accounts.visibilityBusy" :title="t('accountVisibility.hideAccount', { account: account.displayName })" @click="accounts.showToday(account.id, false)">{{ t('accountVisibility.hide') }}</AppButton>
    </div>
  </nav>
</template>

<style scoped>
.sidebar-accounts { display: grid; gap: var(--space-2xs); padding: var(--space-md) var(--space-md) var(--space-xs); }
.sidebar-accounts__row { display: flex; align-items: center; min-width: 0; gap: var(--space-xs); }
.sidebar-accounts__row > :first-child { flex: 1; }
.sidebar-accounts__row > :last-child { flex-shrink: 0; }
.sidebar-accounts__row--resting { color: var(--text-tertiary); }
.sidebar-accounts__row--resting :deep(.nav-item) { color: var(--text-tertiary); }
.sidebar-accounts__identity { display: grid; min-width: 0; }
.sidebar-accounts__identity > span { overflow: hidden; text-overflow: ellipsis; }
.sidebar-accounts__status { font-size: var(--font-size-2xs); font-weight: var(--weight-regular); }
.sidebar-accounts__dot { width: var(--space-sm); height: var(--space-sm); border-radius: var(--radius-full); }
</style>
