<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useUiStore } from '@/stores/ui';
import { useSettingsLocation } from '@/composables/useSettingsLocation';
import { useCertificateTrustStore } from '@/stores/certificateTrust';
import { useAccountQuotaStore } from '@/stores/accountQuota';
import { useWorkflowSyncStore } from '@/stores/workflowSync';
import { certificateShort } from '@/utils/certificateLabels';
import type { Account } from '@/types/mail';
import { providerLabel, syncErrorText, syncProgressLabel, syncStatusLabel } from './accountStatus';
import AccountDetail from './AccountDetail.vue';
import AccountAvatar from '@/components/common/AccountAvatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CardButton from '@/components/ui/CardButton.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';

/**
 * Accounts: a list to see every account at a glance, and one page per
 * account for everything that can be changed about it. The list used to hold
 * every option of every account at once, with no room for renaming one.
 */
const { locale, t } = useI18n();
const accounts = useAccountsStore();
const ui = useUiStore();
const certificates = useCertificateTrustStore();
const quotas = useAccountQuotaStore();
const workflowSync = useWorkflowSyncStore();
const location = useSettingsLocation();
// Quota and workflow sync change at most every few hours; one read per visit.
onMounted(() => {
  void quotas.load();
  void workflowSync.load();
});

const emailAccounts = computed(() => accounts.emailAccounts);
const openAccount = computed(() =>
  emailAccounts.value.find((account) => account.id === location.account.value) ?? null);

type Tone = 'ok' | 'idle' | 'busy' | 'warning' | 'error';

/** The one thing worth knowing about an account, most urgent first. */
function statusOf(account: Account): { tone: Tone; text: string } {
  if (account.lastOauthError) return { tone: 'error', text: t('settings.accounts.status.signInNeeded') };
  if (account.syncStatus === 'error') {
    return { tone: 'error', text: syncErrorText(account) ?? t('settings.accounts.sync.error') };
  }
  const problem = certificates.problemFor(account.id);
  if (problem) return { tone: 'warning', text: certificateShort(problem.rejection) };
  const quota = quotas.levelFor(account.id);
  if (quota === 'warning' || quota === 'critical') {
    return { tone: 'warning', text: quota === 'critical' ? t('accountQuota.critical') : t('accountQuota.warning') };
  }
  if (account.syncStatus === 'syncing') {
    const progress = syncProgressLabel(account.syncProgressJson, t);
    const label = t('settings.accounts.sync.running');
    return { tone: 'busy', text: progress ? `${label} · ${progress}` : label };
  }
  return { tone: account.lastSyncAt ? 'ok' : 'idle', text: syncStatusLabel(account, t, locale.value) };
}
</script>

<template>
  <AccountDetail v-if="openAccount" :key="openAccount.id" :account-id="openAccount.id" />

  <div v-else class="accounts">
    <header class="accounts__header">
      <div class="accounts__heading">
        <h3 class="settings-page-title">{{ $t('settings.sections.accounts.label') }}</h3>
        <p>{{ $t('settings.accounts.description') }}</p>
      </div>
      <AppButton v-if="emailAccounts.length" variant="primary" size="sm" @click="ui.openAccountSetup()">
        <template #icon><AppIcon size="sm"><path d="M8 3v10M3 8h10" /></AppIcon></template>
        {{ $t('settings.accounts.addAccount') }}
      </AppButton>
    </header>

    <EmptyState
      v-if="emailAccounts.length === 0"
      :title="$t('settings.accounts.empty.title')"
      :description="$t('settings.accounts.empty.description')"
      :action-label="$t('settings.accounts.addAccount')"
      @action="ui.openAccountSetup()"
    />

    <section v-else class="accounts__section">
      <SectionHeading :label="$t('settings.accounts.mailAccounts')" :count="emailAccounts.length" />
      <ul class="accounts__list" role="list">
        <li v-for="account in emailAccounts" :key="account.id" class="accounts__item">
          <CardButton
            class="accounts__row"
            :title="$t('settings.accounts.openAccount', { name: account.displayName })"
            data-testid="account-row"
            @click="location.showAccount(account.id)"
          >
            <AccountAvatar
              :name="account.displayName"
              :color="account.color"
              :avatar="account.avatar"
              :size="36"
              :resting="!accounts.isVisible(account.id)"
            />
            <span class="accounts__copy">
              <span class="accounts__name">{{ account.displayName }}</span>
              <span class="accounts__email">{{ account.email }}</span>
              <span class="accounts__status" :class="`accounts__status--${statusOf(account).tone}`">
                <span class="accounts__dot" aria-hidden="true" />
                {{ statusOf(account).text }}
              </span>
            </span>
            <span class="accounts__provider">{{ providerLabel(account.provider) }}</span>
            <AppIcon class="accounts__chevron"><path d="m6 3 5 5-5 5" /></AppIcon>
          </CardButton>
        </li>
      </ul>
    </section>
  </div>
</template>

<style scoped>
.accounts {
  display: grid;
  gap: var(--space-xl);
}

.accounts__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-lg);
}

.accounts__heading {
  display: grid;
  min-width: 0;
}

.accounts__heading p {
  margin: var(--space-2xs) 0 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.accounts__section {
  display: grid;
  gap: var(--space-sm);
}

.accounts__list {
  display: grid;
  margin: 0;
  padding: 0;
  overflow: hidden;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  list-style: none;
  --swatch-ring: var(--bg-surface);
}

.accounts__item + .accounts__item {
  border-top: 1px solid var(--border-color);
}

.accounts__row {
  align-items: center;
  gap: var(--space-md);
  width: 100%;
  padding: var(--space-md) var(--space-lg);
  transition: background var(--duration-fast) var(--ease-out);
}

.accounts__row:hover {
  background: var(--bg-hover);
}

.accounts__copy {
  display: grid;
  flex: 1;
  gap: var(--space-2xs);
  min-width: 0;
}

.accounts__name,
.accounts__email,
.accounts__status {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.accounts__name {
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
}

.accounts__email {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.accounts__status {
  display: flex;
  align-items: center;
  gap: var(--space-1-5);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.accounts__dot {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: var(--radius-full);
  background: var(--accent-green);
}

.accounts__status--idle .accounts__dot { background: var(--text-tertiary); }
.accounts__status--busy { color: var(--accent-primary); }
.accounts__status--busy .accounts__dot { background: var(--accent-primary); }
.accounts__status--warning { color: var(--accent-amber-soft); }
.accounts__status--warning .accounts__dot { background: var(--accent-amber); }
.accounts__status--error { color: var(--accent-red-soft); }
.accounts__status--error .accounts__dot { background: var(--accent-red); }

.accounts__provider {
  flex-shrink: 0;
  padding: var(--space-2xs) var(--space-sm);
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-medium);
}

.accounts__chevron {
  flex-shrink: 0;
  color: var(--text-tertiary);
}

@media (max-width: 767px) {
  .accounts__header {
    flex-direction: column;
  }

  .accounts__provider {
    display: none;
  }
}
</style>
