<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useThreadsStore } from '@/stores/threads';
import { useUiStore } from '@/stores/ui';
import NavItem from '@/components/ui/NavItem.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import AccountAvatar from '@/components/common/AccountAvatar.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useCertificateTrustStore } from '@/stores/certificateTrust';
import { certificateShort, formatCertificateMoment } from '@/utils/certificateLabels';
import { useConnectionStore } from '@/stores/connection';
import { backendErrorKey, backendErrorStringParam } from '@/utils/backendErrors';
import { accountCertificateRejection } from '@/utils/certificateRejection';
import { formatMoment } from '@/utils/listTime';
import type { AnchorBox } from '@/utils/popoverPlacement';

const SidebarAccountMenu = defineAsyncComponent(() => import('./SidebarAccountMenu.vue'));

const { t } = useI18n();
const accounts = useAccountsStore();
const mailboxes = useMailboxesStore();
const threads = useThreadsStore();
const router = useRouter();
const ui = useUiStore();
const certificates = useCertificateTrustStore();
const connection = useConnectionStore();

/** Since when each account's other sync failures have run, from the live run. */
const failingSince = computed(() => {
  const since = new Map<string, string | null>();
  for (const failure of connection.syncFailures) {
    if (backendErrorKey(failure) !== 'backend.errors.accountSyncFailed') continue;
    if (accountCertificateRejection(failure)) continue;
    const accountId = backendErrorStringParam(failure, 'accountId');
    if (accountId) since.set(accountId, backendErrorStringParam(failure, 'since'));
  }
  return since;
});

type Marker = { kind: 'certificate' | 'exception' | 'outage'; label: string };

/**
 * The account row's marker and its tooltip, the short form of the banner. A
 * rejected certificate wins over the generic failure: it has its own dialog.
 */
function markerFor(accountId: string, name: string): Marker | null {
  const problem = certificates.problemFor(accountId);
  if (problem) {
    return { kind: 'certificate', label: t('certificates.sidebar.problem', { account: name, problem: certificateShort(problem.rejection) }) };
  }
  if (failingSince.value.has(accountId)) {
    const since = formatMoment(failingSince.value.get(accountId)) ?? t('common.unknown');
    return { kind: 'outage', label: t('errors.banner.outageShort', { account: name, since }) };
  }
  const exception = certificates.exceptionFor(accountId);
  if (exception) {
    return {
      kind: 'exception',
      label: t('certificates.sidebar.exception', {
        account: name,
        problem: t('certificates.exceptionShort', { until: formatCertificateMoment(exception.expires_at) }),
      }),
    };
  }
  return null;
}

const markers = computed(() => new Map(
  accounts.emailAccounts.map((account) => [account.id, markerFor(account.id, account.displayName)]),
));

function openMarker(accountId: string, marker: Marker) {
  ui.closeMailboxSwitcher();
  if (marker.kind === 'outage') {
    void router.push({ name: 'settings', query: { section: 'accounts', account: accountId } });
  } else {
    certificates.openDetails(accountId);
  }
}

/**
 * An account is a scope, not a place: the view on screen stays and is drawn
 * from the chosen account (Later → Personal's Later). Only a folder, which
 * belongs to one account, falls back to the Inbox. No `navigate` event — the
 * phone sheet stays open so the list behind it can be seen switching.
 */
function open(accountId: string) {
  if (accounts.activeAccountId === accountId) return;
  accounts.setActiveAccount(accountId);
  if (threads.mailboxFilter.kind === 'folder') {
    threads.workflowFilter = 'all';
    threads.setMailboxFilter({ kind: 'section', value: 'inbox' });
  }
  const route = router.currentRoute.value.name;
  if (route !== 'inbox' && route !== 'outbox') void router.push({ name: 'inbox' });
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
  // Beside the row, not under the pointer: opened on top of it, the menu hid
  // the row's own state ("Resting") and the accounts below it. The
  // context-menu key fires at the focused row the same way.
  const row = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuAccountId.value = accountId;
  menuAnchor.value = { left: row.right, top: row.top, right: row.right, bottom: row.top };
}
</script>

<template>
  <nav v-if="shown" class="sidebar-accounts" :aria-label="t('accountVisibility.accounts')">
    <NavItem v-if="accounts.emailAccounts.length > 1" :label="t('folders.allAccounts')" :class="{ 'nav-item--scope': !accounts.activeAccountId }" :aria-current="!accounts.activeAccountId || undefined" :count="inboxCount()" @click="open('')">
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
      <NavItem :label="account.displayName" :class="{ 'nav-item--scope': accounts.activeAccountId === account.id }" :aria-current="accounts.activeAccountId === account.id || undefined" :count="inboxCount(account.id)" :title="accounts.isVisible(account.id) ? t('accountVisibility.openAccount', { account: account.email }) : t('accountVisibility.restingHint', { account: account.email })" @click="open(account.id)">
        <template #leading>
          <AccountAvatar :name="account.displayName" :color="account.color" :avatar="account.avatar" :size="18" :resting="!accounts.isVisible(account.id)" />
        </template>
        <!-- Resting: still listed, but asleep. No number pulls at you; a moon
             (Lucide `moon`) sits where it would be. -->
        <template v-if="!accounts.isVisible(account.id)" #trailing>
          <span class="sidebar-accounts__asleep" role="img" :aria-label="t('accountVisibility.resting')">
            <span class="sidebar-accounts__asleep-label" aria-hidden="true">{{ t('accountVisibility.resting') }}</span>
            <AppIcon :grid="24"><path d="M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401" /></AppIcon>
          </span>
        </template>
        <!-- Shown for today against its schedule: a clock beside the count says
             the number is borrowed until midnight. -->
        <template v-else-if="accounts.isPeeked(account.id)" #trailing>
          <span class="sidebar-accounts__peek" role="img" :aria-label="t('accountVisibility.statePeek')" :title="t('accountVisibility.statePeek')">
            <AppIcon><circle cx="8" cy="8" r="5.6" /><path d="M8 4.8V8l2.3 1.4" /></AppIcon>
          </span>
          <span v-if="inboxCount(account.id) > 0" class="sidebar-accounts__count">{{ inboxCount(account.id) }}</span>
        </template>
      </NavItem>
      <!-- A sibling, not inside NavItem: that is a button already. Orange while
           the account cannot sync (a rejected certificate opens its details,
           anything else the account settings); a quiet shield while a
           certificate exception lets it through. -->
      <IconButton
        v-if="markers.get(account.id)"
        class="sidebar-accounts__certificate"
        :class="{ 'sidebar-accounts__certificate--problem': markers.get(account.id)!.kind !== 'exception' }"
        :size="ui.isMobile ? 'lg' : 'sm'"
        :title="markers.get(account.id)!.label"
        @click="openMarker(account.id, markers.get(account.id)!)"
      >
        <AppIcon v-if="markers.get(account.id)!.kind !== 'exception'">
          <path d="M8 2.4 14.2 13H1.8L8 2.4Z" />
          <path d="M8 6.6v3" />
          <path d="M8 11.6h.01" />
        </AppIcon>
        <AppIcon v-else>
          <path d="M8 1.8 2.8 3.8v3.7c0 3.2 2.2 5.5 5.2 6.4 3-.9 5.2-3.2 5.2-6.4V3.8L8 1.8Z" />
          <path d="M8 5.6v2.5l1.6 1" />
        </AppIcon>
      </IconButton>
    </div>
    <SidebarAccountMenu v-if="menuAccountId" :account-id="menuAccountId" :anchor="menuAnchor" @close="menuAnchor = null" />
  </nav>
</template>

<style scoped>
/* One lightly tinted surface holds the scope choice, edge to edge with the
   view rows below it, so both its edges and its rows' icons and labels line
   up with theirs. */
.sidebar-accounts {
  display: grid;
  gap: var(--space-2xs);
  margin: var(--space-md) var(--space-md) var(--space-xs);
  padding: var(--space-xs) 0;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
}
.sidebar-accounts__row { display: flex; align-items: center; min-width: 0; }
.sidebar-accounts__row > :first-child { flex: 1; }
.sidebar-accounts__row--resting { color: var(--text-tertiary); }
.sidebar-accounts__row--resting :deep(.nav-item) { color: var(--text-tertiary); }
.sidebar-accounts__asleep { display: inline-flex; align-items: center; gap: var(--space-xs); color: var(--text-tertiary); }
.sidebar-accounts__asleep-label { font-size: var(--font-size-xs); }
.sidebar-accounts__peek { display: inline-flex; color: var(--text-accent); }
/* Prefixed with the row to outrank IconButton's own colours. */
.sidebar-accounts__row .sidebar-accounts__certificate { color: var(--text-tertiary); }
.sidebar-accounts__row .sidebar-accounts__certificate--problem { color: var(--accent-orange); }
@media (hover: hover) {
  .sidebar-accounts__row .sidebar-accounts__certificate--problem:hover:not(:disabled) {
    color: var(--accent-orange);
    background: color-mix(in srgb, var(--accent-orange) 12%, transparent);
  }
}
/* NavItem's own count, kept beside the clock that took its slot. */
.sidebar-accounts__count {
  margin-left: var(--space-xs);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

/* Rings cut each avatar out of the one behind it, in the surface's colour. */
.sidebar-accounts { --avatar-ring: var(--bg-raised); }
/* A tap leaves :hover behind on touch screens; it would read as a selection. */
@media (hover: hover) {
  .sidebar-accounts :deep(.nav-item:hover) { background: color-mix(in srgb, var(--text-primary) 6%, transparent); }
}
@media (hover: none) {
  .sidebar-accounts :deep(.nav-item:hover) { background: transparent; }
}

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
