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
 *
 * A rejected server certificate is the server's fault, not the person's: it
 * gets the warning tone, names the account and the date, and can be hidden.
 * Hidden stays hidden for that certificate; the account keeps its marker in
 * the sidebar, and a different certificate is a new notice.
 */
import { computed, defineAsyncComponent, onBeforeUnmount, ref, watchEffect } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { backendErrorLabel, backendErrorSourceLabel } from '@/utils/backendErrorLabels';
import { describeSyncFailure, isOutage } from '@/utils/syncFailures';
import { formatMoment } from '@/utils/listTime';
import { accountCertificateRejection } from '@/utils/certificateRejection';
import { certificateDetail, certificateShort, certificateTitle } from '@/utils/certificateLabels';
import { useCertificateTrustStore } from '@/stores/certificateTrust';
import { useUiStore } from '@/stores/ui';
import AppBanner from '@/components/ui/AppBanner.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';

// Mounted here rather than in App: this banner already loads on demand and is
// always present, and the certificate code stays out of the startup bundle.
// The sidebar marker and the account settings open the same dialog.
const CertificateDialog = defineAsyncComponent(() => import('./CertificateDialog.vue'));

const connection = useConnectionStore();
const accounts = useAccountsStore();
const certificates = useCertificateTrustStore();
const ui = useUiStore();
const router = useRouter();
const { t } = useI18n();

const passwordAccounts = computed(() => new Set(
  accounts.accounts
    .filter((account) => account.provider === 'imap' || account.provider === 'pop3')
    .map((account) => account.id),
));
const isSync = computed(() => connection.backendErrorSource === 'sync');
// Certificate failures have their own banner below; everything else keeps this one.
const syncFailures = computed(() => (
  isSync.value
    ? connection.syncFailures
      .filter((failure) => !accountCertificateRejection(failure))
      .map((failure) => describeSyncFailure(failure, passwordAccounts.value))
    : []
));
const onlyCertificates = computed(() => isSync.value
  && connection.syncFailures.length > 0
  && syncFailures.value.length === 0);
const certificateProblem = computed(() => (onlyCertificates.value ? certificates.visibleProblems[0] ?? null : null));
const certificateAccount = computed(() => {
  const id = certificateProblem.value?.accountId;
  const account = accounts.accounts.find((entry) => entry.id === id);
  return account?.displayName || account?.email || '';
});
const rejected = computed(() => {
  const byAccount = new Map(
    syncFailures.value
      .filter((failure) => failure.passwordRejected && failure.accountId)
      .map((failure) => [failure.accountId!, failure]),
  );
  return [...byAccount.values()];
});

/**
 * One account that has not synced for an hour or more is an outage, not a
 * hiccup: the headline names it and says since when. The October outage ran
 * for two days and was noticed only because no mail arrived.
 */
// Sync reports arrive every few seconds while it polls, but not during a
// backoff; the clock makes the headline turn at the hour regardless.
const now = ref(Date.now());
const clock = setInterval(() => { now.value = Date.now(); }, 60_000);
onBeforeUnmount(() => clearInterval(clock));

/** Failures that belong to an account. A blocked folder also reports a bare entry of its own. */
const accountFailures = computed(() => syncFailures.value.filter((failure) => failure.accountId));

const outage = computed(() => {
  const accountIds = new Set(accountFailures.value.map((failure) => failure.accountId));
  const [first] = accountFailures.value;
  if (accountIds.size !== 1 || !first?.accountId || !isOutage(first, now.value)) return null;
  const account = accounts.accounts.find((entry) => entry.id === first.accountId);
  return {
    accountId: first.accountId,
    account: account?.displayName || account?.email || first.account,
    since: formatMoment(first.since) ?? '',
  };
});

const title = computed(() => (outage.value
  ? t('errors.banner.accountOutage', { account: outage.value.account, since: outage.value.since })
  : backendErrorSourceLabel(connection.backendErrorSource)));

// The drawer handle on a phone: a dot while a certificate is rejected or an
// account has been failing for an hour, so the marker behind the closed
// drawer is not the only sign. A failure a minute old gets no dot: the handle
// would flicker with every hiccup. Owned here because this banner is always
// mounted and already knows both kinds of failure.
watchEffect(() => {
  ui.drawerAttention = certificates.needsAttention || connection.syncFailures.some((raw) => {
    if (accountCertificateRejection(raw)) return false;
    const failure = describeSyncFailure(raw, passwordAccounts.value);
    return failure.accountId !== null && isOutage(failure, now.value);
  });
});
const detail = computed(() => (
  syncFailures.value.length > 0
    ? syncFailures.value.map((failure) => failure.label).join(' · ')
    : backendErrorLabel(connection.backendError)
));

/** The account's settings: password, server, and the last error in full. */
function updatePassword(accountId: string) {
  void router.push({ name: 'settings', query: { section: 'accounts', account: accountId } });
}
</script>

<template>
  <AppBanner
    v-if="certificateProblem"
    class="connection-banner connection-banner--certificate"
    :class="{ 'connection-banner--compact': ui.isMobile }"
    tone="warning"
    :title="ui.isMobile
      ? t('certificates.sidebar.problem', { account: certificateAccount, problem: certificateShort(certificateProblem.rejection) })
      : certificateTitle(certificateProblem.rejection, certificateAccount)"
    :detail="ui.isMobile ? undefined : certificateDetail(certificateProblem.rejection)"
  >
    <template #actions>
      <template v-if="ui.isMobile">
        <AppButton size="sm" variant="soft" @click="certificates.openDetails(certificateProblem.accountId)">
          {{ t('certificates.banner.details') }}
        </AppButton>
        <IconButton size="sm" :title="t('certificates.banner.hide')" @click="certificates.dismiss(certificateProblem.accountId)">
          <AppIcon><path d="m4 4 8 8m0-8-8 8" /></AppIcon>
        </IconButton>
      </template>
      <template v-else>
        <AppButton size="sm" variant="ghost" @click="certificates.dismiss(certificateProblem.accountId)">
          {{ t('certificates.banner.hide') }}
        </AppButton>
        <AppButton size="sm" variant="soft" @click="certificates.openDetails(certificateProblem.accountId)">
          {{ t('certificates.banner.details') }}
        </AppButton>
        <AppButton size="sm" variant="primary" :disabled="connection.retrying" @click="connection.requestRetry()">
          {{ connection.retrying ? t('errors.banner.retrying') : t('common.retry') }}
        </AppButton>
      </template>
    </template>
  </AppBanner>
  <AppBanner
    v-else-if="connection.backendError && !onlyCertificates"
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
        v-if="outage && rejected.length === 0"
        size="sm"
        variant="ghost"
        @click="updatePassword(outage.accountId)"
      >
        {{ t('errors.banner.openAccount') }}
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
  <CertificateDialog v-if="certificates.detailsAccountId" />
</template>

<style scoped>
/* The shell is a flex column; without this the banner is squeezed to nothing. */
.connection-banner {
  flex-shrink: 0;
}

/* Orange rather than the amber warning: the same colour as the account's
   marker in the sidebar, so banner and marker read as one fact. */
.connection-banner--certificate {
  background: color-mix(in srgb, var(--accent-orange) 10%, var(--bg-surface));
  border-bottom-color: color-mix(in srgb, var(--accent-orange) 26%, transparent);
}

.connection-banner--certificate :deep(.banner__icon) {
  color: var(--accent-orange);
}

/* The sentence says who has to act and since when; it may take two lines
   rather than lose its end to an ellipsis, as the raw error did. */
.connection-banner--certificate :deep(.banner__detail) {
  white-space: normal;
}

/* The phone keeps one line: the title, Details and the cross. */
.connection-banner--compact :deep(.banner__title) {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.connection-banner--compact :deep(.banner__copy) {
  min-width: 0;
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
