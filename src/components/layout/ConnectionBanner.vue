<script setup lang="ts">
/**
 * The standing half of the backend error channel.
 *
 * Every subsystem already reports into `connection.reportBackendError`, but
 * until this banner existed nothing read that store — a failed thread load
 * looked exactly like an empty mailbox. Anything that is over by the time it is
 * shown belongs in `useToast().error` instead; this one stays until the cause
 * is gone, because the problem does.
 *
 * A rejected password is the one sync failure the person has to fix, so it
 * says so and offers the way there; the server's own words stay one click away.
 */
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { backendErrorLabel, backendErrorSourceLabel } from '@/utils/backendErrorLabels';
import { describeSyncFailure } from '@/utils/syncFailures';
import AppBanner from '@/components/ui/AppBanner.vue';
import AppButton from '@/components/ui/AppButton.vue';

const connection = useConnectionStore();
const accounts = useAccountsStore();
const router = useRouter();
const { t } = useI18n();

const passwordAccounts = computed(() => new Set(
  accounts.accounts
    .filter((account) => account.provider === 'imap' || account.provider === 'pop3')
    .map((account) => account.id),
));
const syncFailures = computed(() => (
  connection.backendErrorSource === 'sync'
    ? connection.syncFailures.map((failure) => describeSyncFailure(failure, passwordAccounts.value))
    : []
));
const rejected = computed(() => {
  const byAccount = new Map(
    syncFailures.value
      .filter((failure) => failure.passwordRejected && failure.accountId)
      .map((failure) => [failure.accountId!, failure]),
  );
  return [...byAccount.values()];
});

const title = computed(() => backendErrorSourceLabel(connection.backendErrorSource));
const detail = computed(() => (
  syncFailures.value.length > 0
    ? syncFailures.value.map((failure) => failure.label).join(' · ')
    : backendErrorLabel(connection.backendError)
));

function updatePassword(accountId: string) {
  void router.push({ name: 'settings', query: { section: 'accounts', account: accountId } });
}
</script>

<template>
  <AppBanner
    v-if="connection.backendError"
    class="connection-banner"
    tone="danger"
    :title="title"
    :detail="detail"
  >
    <details v-if="rejected.length > 0" class="connection-banner__server">
      <summary>{{ t('errors.banner.serverResponse') }}</summary>
      <code v-for="failure in rejected" :key="failure.accountId!">{{ failure.serverText }}</code>
    </details>
    <template #actions>
      <AppButton
        v-for="failure in rejected"
        :key="failure.accountId!"
        size="sm"
        variant="primary"
        :title="failure.account"
        @click="updatePassword(failure.accountId!)"
      >
        {{ t('errors.banner.updatePassword') }}
      </AppButton>
      <AppButton
        size="sm"
        :variant="rejected.length > 0 ? 'ghost' : 'primary'"
        :disabled="connection.retrying"
        @click="connection.requestRetry()"
      >
        {{ connection.retrying ? t('errors.banner.retrying') : t('common.retry') }}
      </AppButton>
    </template>
  </AppBanner>
</template>

<style scoped>
/* The shell is a flex column; without this the banner is squeezed to nothing. */
.connection-banner {
  flex-shrink: 0;
}

.connection-banner__server {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.connection-banner__server[open] {
  flex-basis: 100%;
}

.connection-banner__server summary {
  cursor: pointer;
}

.connection-banner__server code {
  display: block;
  margin-top: var(--space-2xs);
  font-family: var(--font-mono);
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}
</style>
