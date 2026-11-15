<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useThreadsStore } from '@/stores/threads';
import { useCertificateTrustStore } from '@/stores/certificateTrust';
import { useAccountQuotaStore } from '@/stores/accountQuota';
import { useSettingsLocation } from '@/composables/useSettingsLocation';
import { certificateShort, formatCertificateMoment } from '@/utils/certificateLabels';
import {
  backendAutoconfigureGroupware,
  backendDeleteAccount,
  backendCommandsAvailable,
  backendConfigureGroupware,
  backendOauthStatus,
  backendRunImapSync,
  backendSyncGroupware,
  backendTestImapAccountConnection,
  backendUpdateAccountPassword,
  translateBackendError,
} from '@/services/backend';
import { useConfirm } from '@/composables/useConfirm';
import { transportErrorLabel, transportErrorText } from '@/utils/transportLabels';
import {
  providerLabel,
  syncErrorText,
  syncProgressLabel,
  syncStatusLabel,
  usesOauth,
  usesPassword,
} from './accountStatus';
import AccountNameSettings from './AccountNameSettings.vue';
import AccountVisibilitySettings from './AccountVisibilitySettings.vue';
import AccountAppearanceSettings from './AccountAppearanceSettings.vue';
import AccountQuotaSettings from './AccountQuotaSettings.vue';
import AccountWorkflowSyncNote from './AccountWorkflowSyncNote.vue';
import AccountAvatar from '@/components/common/AccountAvatar.vue';
import AppBanner from '@/components/ui/AppBanner.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';

// Both talk to the server when they open; only the accounts that have them load them.
const ProviderFeaturesEditor = defineAsyncComponent(() => import('./ProviderFeaturesEditor.vue'));
const ServerRulesSettings = defineAsyncComponent(() => import('./ServerRulesSettings.vue'));

const props = defineProps<{ accountId: string }>();

const { locale, t } = useI18n();
const accounts = useAccountsStore();
const connection = useConnectionStore();
const mailboxes = useMailboxesStore();
const threads = useThreadsStore();
const certificates = useCertificateTrustStore();
const quotas = useAccountQuotaStore();
const location = useSettingsLocation();
const { ask } = useConfirm();
const commandsAvailable = backendCommandsAvailable();

const account = computed(() => accounts.emailAccounts.find((item) => item.id === props.accountId) ?? null);
const hasQuota = computed(() => quotas.quotaFor(props.accountId) != null);

const certificateLine = computed(() => {
  const problem = certificates.problemFor(props.accountId);
  if (problem) return certificateShort(problem.rejection);
  const exception = certificates.exceptionFor(props.accountId);
  return exception
    ? t('certificates.exceptionShort', { until: formatCertificateMoment(exception.expires_at) })
    : null;
});

const syncLine = computed(() => {
  if (!account.value) return '';
  const progress = syncProgressLabel(account.value.syncProgressJson, t);
  const label = syncStatusLabel(account.value, t, locale.value);
  return progress ? `${label} · ${progress}` : label;
});
const syncError = computed(() => (account.value ? syncErrorText(account.value) : null));

const syncBusy = ref(false);
const statusText = ref('');
/**
 * The untouched chain behind the last failure, paired with the line it belongs
 * to, so stale details cannot outlive the failure.
 */
const lastFailure = ref({ status: '', raw: '' });
const statusDetail = computed(() =>
  lastFailure.value.status === statusText.value ? lastFailure.value.raw : '',
);
const statusIsError = computed(() =>
  statusText.value !== '' && lastFailure.value.status === statusText.value,
);

function reportProblem(text: string) {
  statusText.value = text;
  lastFailure.value = { status: text, raw: '' };
}

/** Put the class of failure in the status line and the chain behind it. */
function reportFailure(key: string, error: unknown) {
  const raw = transportErrorText(error);
  const label = transportErrorLabel(error);
  statusText.value = t(key, { error: label || raw });
  lastFailure.value = { status: statusText.value, raw: label === raw ? '' : raw };
}

/* --- Password ---------------------------------------------------------- */

const passwordOpen = ref(false);
const newPassword = ref('');
const passwordBusy = ref(false);
const passwordInput = ref<HTMLInputElement | null>(null);

function openPasswordForm() {
  passwordOpen.value = true;
  newPassword.value = '';
  // Focusing scrolls the field into view as well.
  void nextTick(() => passwordInput.value?.focus());
}

function togglePasswordForm() {
  if (passwordOpen.value) passwordOpen.value = false;
  else openPasswordForm();
}

// The connection banner sends a failing password account here; the form is
// what it came for.
onMounted(() => {
  if (account.value && usesPassword(account.value.provider) && account.value.syncStatus === 'error') {
    openPasswordForm();
  }
});

async function savePassword() {
  const current = account.value;
  if (!current || !newPassword.value || passwordBusy.value) return;
  passwordBusy.value = true;
  try {
    await backendUpdateAccountPassword(current.id, newPassword.value);
    newPassword.value = '';
    passwordOpen.value = false;
    statusText.value = t('settings.accounts.password.saved', { email: current.email });
    connection.requestRetry();
    void accounts.load();
  } catch (error) {
    reportFailure('settings.accounts.password.failed', error);
  } finally {
    passwordBusy.value = false;
  }
}

/* --- Calendars and contacts (CalDAV / CardDAV) ------------------------- */

const groupwareOpen = ref(false);
const groupwareBusy = ref(false);
const groupwareUsername = ref('');
const groupwarePassword = ref('');
const caldavServer = ref('');
const carddavServer = ref('');

function groupwareErrorSuffix(errors: string[]): string {
  if (errors.length === 0) return '';
  return `; ${errors.map((error) => translateBackendError(error) ?? error).join('; ')}`;
}

function openGroupwareForm() {
  const email = account.value?.email ?? '';
  const domain = email.split('@')[1]?.trim() ?? '';
  groupwareUsername.value = email.split('@')[0]?.trim() ?? email;
  groupwarePassword.value = '';
  caldavServer.value = domain ? `gwcaldav.${domain}` : '';
  carddavServer.value = domain ? `gwcarddav.${domain}` : '';
  groupwareOpen.value = true;
}

async function connectGroupware() {
  if (groupwareOpen.value) {
    groupwareOpen.value = false;
    return;
  }
  groupwareBusy.value = true;
  statusText.value = t('settings.accounts.groupware.discovering');
  try {
    const result = await backendAutoconfigureGroupware(props.accountId);
    let services = t('settings.accounts.groupware.contacts');
    if (result.caldav_server && result.carddav_server) {
      services = t('settings.accounts.groupware.calendarAndContacts');
    } else if (result.caldav_server) {
      services = t('settings.accounts.groupware.calendar');
    }
    statusText.value = t('settings.accounts.groupware.connected', {
      services,
      events: result.summary.events_synced,
      contacts: result.summary.contacts_synced,
      suffix: groupwareErrorSuffix(result.summary.errors),
    });
  } catch (error) {
    reportFailure('settings.accounts.groupware.autoFailed', error);
    openGroupwareForm();
  } finally {
    groupwareBusy.value = false;
  }
}

async function saveGroupware() {
  if (!groupwareUsername.value.trim() || !groupwarePassword.value) {
    reportProblem(t('settings.accounts.groupware.credentialsRequired'));
    return;
  }
  if (!caldavServer.value.trim() && !carddavServer.value.trim()) {
    reportProblem(t('settings.accounts.groupware.serverRequired'));
    return;
  }
  groupwareBusy.value = true;
  try {
    const result = await backendConfigureGroupware({
      accountId: props.accountId,
      username: groupwareUsername.value.trim(),
      password: groupwarePassword.value,
      caldavServer: caldavServer.value.trim() || null,
      carddavServer: carddavServer.value.trim() || null,
    });
    statusText.value = t('settings.accounts.groupware.saved', {
      events: result.events_synced,
      contacts: result.contacts_synced,
      suffix: groupwareErrorSuffix(result.errors),
    });
    groupwarePassword.value = '';
    groupwareOpen.value = false;
  } catch (error) {
    reportFailure('settings.accounts.groupware.saveFailed', error);
  } finally {
    groupwareBusy.value = false;
  }
}

async function syncGroupwareNow() {
  groupwareBusy.value = true;
  try {
    const result = await backendSyncGroupware(props.accountId);
    statusText.value = t('settings.accounts.groupware.synced', {
      events: result.events_synced,
      contacts: result.contacts_synced,
      suffix: groupwareErrorSuffix(result.errors),
    });
  } catch (error) {
    reportFailure('settings.accounts.groupware.syncFailed', error);
  } finally {
    groupwareBusy.value = false;
  }
}

/* --- Sync, test, sign-in ----------------------------------------------- */

async function syncImapNow() {
  if (!commandsAvailable) {
    reportProblem(t('settings.accounts.sync.requiresApp'));
    return;
  }
  syncBusy.value = true;
  try {
    const summary = await backendRunImapSync();
    // Same numbers this pane prints, for the inbox that has no reply of its own.
    connection.applySyncSummary(summary);
    await Promise.all([threads.load(), mailboxes.load(), accounts.load()]);
    const messages = t('settings.accounts.sync.messagesCount', { count: summary.messages_synced }, summary.messages_synced);
    const localThreads = threads.threads.filter((thread) => thread.accountId === props.accountId).length;
    const threadsCount = t('settings.accounts.sync.threadsCount', { count: localThreads }, localThreads);
    if (summary.failures.length > 0 || summary.accounts_failed > 0) {
      const rawFailure = summary.failures[0];
      reportProblem(t('settings.accounts.sync.partial', {
        messages,
        failed: Math.max(summary.accounts_failed, summary.failures.length),
        error: rawFailure ? translateBackendError(rawFailure) ?? rawFailure : t('settings.accounts.sync.unknownFailure'),
        threads: threadsCount,
      }));
    } else if (summary.messages_synced === 0) {
      statusText.value = t('settings.accounts.sync.finishedEmpty', { threads: threadsCount });
    } else {
      statusText.value = t('settings.accounts.sync.finished', {
        messages,
        accounts: t('settings.accounts.sync.accountsCount', { count: summary.accounts_synced }, summary.accounts_synced),
        threads: threadsCount,
      });
    }
  } catch (error) {
    reportFailure('settings.accounts.sync.failed', error);
  } finally {
    syncBusy.value = false;
  }
}

const testTitle = computed(() => {
  const provider = account.value?.provider;
  if (provider === 'imap') return t('settings.accounts.testImap');
  if (provider === 'pop3') return t('settings.accounts.testPop3');
  return t('settings.accounts.checkOrRenew');
});

async function checkStatus() {
  const current = account.value;
  if (!current) return;
  if (current.provider === 'imap') {
    try {
      const result = await backendTestImapAccountConnection(current.id);
      const sample = result.folder_sample.slice(0, 3).join(', ');
      statusText.value = sample
        ? t('settings.accounts.imapOkWithSample', { folders: result.folders, sample })
        : t('settings.accounts.imapOk', { folders: result.folders });
    } catch (error) {
      reportFailure('settings.accounts.imapFailed', error);
    }
    return;
  }
  try {
    const result = await backendOauthStatus(current.id);
    statusText.value = t('settings.accounts.oauthStatus', {
      provider: result.provider,
      state: result.connected ? t('settings.accounts.connected') : t('settings.accounts.disconnected'),
    });
  } catch (error) {
    reportFailure('settings.accounts.statusUnavailable', error);
  }
}

const reauthBusy = ref(false);

async function reconnectOauth() {
  const current = account.value;
  if (!current || !commandsAvailable || reauthBusy.value) return;
  reauthBusy.value = true;
  statusText.value = t('settings.accounts.reconnect.browserOpened', { email: current.email });
  try {
    await accounts.startReauth(current.id);
    statusText.value = t('settings.accounts.reconnect.done', { email: current.email });
  } catch (error) {
    reportFailure('settings.accounts.reconnect.failed', error);
  } finally {
    reauthBusy.value = false;
  }
}

async function removeAccount() {
  const current = account.value;
  if (!current) return;
  if (!commandsAvailable) {
    reportProblem(t('settings.accounts.removeRequiresApp'));
    return;
  }
  const confirmed = await ask({
    title: t('settings.accounts.removeConfirmTitle'),
    body: t('settings.accounts.removeConfirmBody'),
    detail: current.email,
    confirmLabel: t('settings.accounts.removeConfirmAction'),
    tone: 'danger',
  });
  if (!confirmed) return;
  try {
    await backendDeleteAccount(current.id);
    await accounts.load();
    await Promise.all([threads.load(), mailboxes.load()]);
    location.showAccount(null);
  } catch (error) {
    reportFailure('settings.accounts.removeFailed', error);
  }
}
</script>

<template>
  <div v-if="account" class="account-detail">
    <AppButton
      v-if="location.inDialog.value"
      class="account-detail__back"
      variant="link"
      size="sm"
      @click="location.showAccount(null)"
    >
      <template #icon><AppIcon size="sm"><path d="m10 3-5 5 5 5" /></AppIcon></template>
      {{ $t('settings.accounts.allAccounts') }}
    </AppButton>

    <header class="account-detail__hero">
      <AccountAvatar :name="account.displayName" :color="account.color" :avatar="account.avatar" :size="48" />
      <div class="account-detail__identity">
        <h3 class="settings-page-title">{{ account.displayName }}</h3>
        <p>{{ account.email }} · {{ providerLabel(account.provider) }}</p>
      </div>
    </header>

    <div v-if="account.lastOauthError || certificateLine" class="account-detail__notices">
      <AppBanner
        v-if="account.lastOauthError"
        variant="card"
        tone="danger"
        :title="$t('settings.accounts.status.signInNeeded')"
        :detail="translateBackendError(account.lastOauthError) ?? account.lastOauthError"
      >
        <template #actions>
          <AppButton size="sm" variant="primary" :disabled="!commandsAvailable || reauthBusy" @click="reconnectOauth">
            {{ $t('settings.accounts.reauth.action') }}
          </AppButton>
        </template>
      </AppBanner>
      <AppBanner v-if="certificateLine" variant="card" tone="warning" :title="certificateLine">
        <template #actions>
          <AppButton size="sm" @click="certificates.openDetails(account.id)">{{ $t('common.show') }}</AppButton>
        </template>
      </AppBanner>
    </div>
    <AccountWorkflowSyncNote :account-id="account.id" />

    <section class="account-detail__section">
      <SectionHeading :label="$t('settings.accounts.detail.general')" />
      <div class="setting-group">
        <AccountNameSettings :account-id="account.id" />
        <div class="account-detail__block">
          <AccountAppearanceSettings :account-id="account.id" />
        </div>
      </div>
    </section>

    <section class="account-detail__section">
      <SectionHeading :label="$t('settings.accounts.detail.connection')" />
      <div class="setting-group">
        <div class="setting-row setting-row--stack-phone">
          <div class="setting-info">
            <span class="setting-label">{{ $t('settings.accounts.detail.syncTitle') }}</span>
            <span :class="{ 'account-detail__busy': account.syncStatus === 'syncing' }">{{ syncLine }}</span>
            <span v-if="syncError" class="account-detail__error">{{ syncError }}</span>
          </div>
          <div class="account-detail__actions">
            <AppButton
              v-if="account.provider === 'imap'"
              variant="secondary"
              size="sm"
              :disabled="!commandsAvailable || syncBusy"
              @click="syncImapNow"
            >
              {{ syncBusy ? $t('settings.accounts.sync.syncing') : $t('settings.accounts.actions.sync') }}
            </AppButton>
            <AppButton variant="secondary" size="sm" :title="testTitle" :disabled="!commandsAvailable" @click="checkStatus">
              {{ $t('settings.accounts.actions.test') }}
            </AppButton>
          </div>
        </div>

        <template v-if="usesPassword(account.provider)">
          <div class="setting-row setting-row--stack-phone">
            <div class="setting-info">
              <span class="setting-label">{{ $t('settings.accounts.detail.passwordTitle') }}</span>
              <span>{{ $t('settings.accounts.password.formCopy') }}</span>
            </div>
            <AppButton variant="secondary" size="sm" :disabled="!commandsAvailable" @click="togglePasswordForm">
              {{ $t('settings.accounts.password.action') }}
            </AppButton>
          </div>
          <form v-if="passwordOpen" class="account-detail__form password-form" @submit.prevent="savePassword">
            <label class="field">
              <span class="field-label">{{ $t('settings.accounts.password.formTitle', { email: account.email }) }}</span>
              <input
                ref="passwordInput"
                v-model="newPassword"
                type="password"
                autocomplete="current-password"
                :placeholder="$t('settings.accounts.password.placeholder')"
              />
            </label>
            <div class="form-actions">
              <AppButton size="sm" :disabled="passwordBusy" @click="passwordOpen = false">{{ $t('common.cancel') }}</AppButton>
              <AppButton type="submit" size="sm" variant="primary" :disabled="passwordBusy || !newPassword">
                {{ passwordBusy ? $t('settings.accounts.password.checking') : $t('settings.accounts.password.save') }}
              </AppButton>
            </div>
          </form>
        </template>

        <div v-if="usesOauth(account.provider)" class="setting-row setting-row--stack-phone">
          <div class="setting-info">
            <span class="setting-label">{{ $t('settings.accounts.detail.signInTitle') }}</span>
            <span>{{ $t('settings.accounts.detail.signInHint', { provider: providerLabel(account.provider) }) }}</span>
          </div>
          <AppButton
            variant="secondary"
            size="sm"
            :title="$t('settings.accounts.reconnect.buttonTitle')"
            :disabled="!commandsAvailable || reauthBusy"
            @click="reconnectOauth"
          >
            {{ $t('settings.accounts.actions.renew') }}
          </AppButton>
        </div>

        <template v-if="account.provider === 'imap'">
          <div class="setting-row setting-row--stack-phone">
            <div class="setting-info">
              <span class="setting-label">{{ $t('settings.accounts.actions.calendars') }}</span>
              <span>{{ $t('settings.accounts.detail.calendarsHint') }}</span>
            </div>
            <AppButton
              variant="secondary"
              size="sm"
              :title="$t('settings.accounts.groupware.connectTitle')"
              :disabled="!commandsAvailable || groupwareBusy"
              @click="connectGroupware"
            >
              {{ $t('settings.accounts.groupware.connect') }}
            </AppButton>
          </div>
          <div v-if="groupwareOpen" class="account-detail__form groupware-form">
            <p class="account-detail__form-title">{{ $t('settings.accounts.groupware.formTitle', { email: account.email }) }}</p>
            <p class="account-detail__form-copy">{{ $t('settings.accounts.groupware.formCopy') }}</p>
            <div class="field-row">
              <label class="field">
                <span class="field-label">{{ $t('settings.accounts.groupware.username') }}</span>
                <input v-model="groupwareUsername" autocomplete="username" :placeholder="$t('settings.accounts.groupware.usernamePlaceholder')" />
              </label>
              <label class="field">
                <span class="field-label">{{ $t('settings.accounts.groupware.password') }}</span>
                <input v-model="groupwarePassword" type="password" autocomplete="current-password" :placeholder="$t('settings.accounts.groupware.passwordPlaceholder')" />
              </label>
            </div>
            <div class="field-row">
              <label class="field">
                <span class="field-label">{{ $t('settings.accounts.groupware.caldavServer') }}</span>
                <input v-model="caldavServer" :placeholder="$t('settings.accounts.groupware.caldavServerPlaceholder')" />
              </label>
              <label class="field">
                <span class="field-label">{{ $t('settings.accounts.groupware.carddavServer') }}</span>
                <input v-model="carddavServer" :placeholder="$t('settings.accounts.groupware.carddavServerPlaceholder')" />
              </label>
            </div>
            <div class="form-actions">
              <AppButton size="sm" :disabled="groupwareBusy" @click="syncGroupwareNow">{{ $t('settings.accounts.groupware.syncNow') }}</AppButton>
              <AppButton size="sm" variant="primary" :disabled="groupwareBusy" @click="saveGroupware">
                {{ groupwareBusy ? $t('settings.accounts.groupware.checking') : $t('settings.accounts.groupware.connect') }}
              </AppButton>
            </div>
          </div>
        </template>
      </div>

      <p
        v-if="statusText"
        class="status-msg"
        :class="{ 'status-msg--error': statusIsError }"
        :role="statusIsError ? 'alert' : 'status'"
      >
        {{ statusText }}
      </p>
      <details v-if="statusDetail" class="status-detail">
        <summary>{{ $t('errors.technicalDetails') }}</summary>
        <pre>{{ statusDetail }}</pre>
      </details>
    </section>

    <section v-if="hasQuota" class="account-detail__section">
      <div class="setting-group account-detail__block">
        <AccountQuotaSettings :account-id="account.id" />
      </div>
    </section>

    <section class="account-detail__section">
      <div class="setting-group account-detail__block">
        <AccountVisibilitySettings :account-id="account.id" />
      </div>
    </section>

    <section v-if="account.provider === 'imap'" class="account-detail__section">
      <SectionHeading :label="$t('settings.serverRules.title')" />
      <p class="account-detail__hint">{{ $t('settings.serverRules.hint') }}</p>
      <ServerRulesSettings :account-id="account.id" />
    </section>

    <section v-if="usesOauth(account.provider)" class="account-detail__section">
      <SectionHeading :label="$t('settings.accounts.detail.labelsTitle')" />
      <p class="account-detail__hint">{{ $t('settings.providerFeatures.hint') }}</p>
      <ProviderFeaturesEditor :account-id="account.id" />
    </section>

    <section class="account-detail__section">
      <div class="setting-group">
        <div class="setting-row setting-row--stack-phone">
          <div class="setting-info">
            <span class="setting-label">{{ $t('settings.accounts.detail.remove') }}</span>
            <span>{{ $t('settings.accounts.detail.removeHint') }}</span>
          </div>
          <AppButton variant="danger" size="sm" :disabled="!commandsAvailable" @click="removeAccount">
            {{ $t('settings.accounts.removeTitle') }}
          </AppButton>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped src="./settingsRows.css"></style>
<style scoped>
.account-detail {
  display: grid;
  gap: var(--space-xl);
}

.account-detail__back {
  justify-self: start;
  margin-bottom: calc(-1 * var(--space-md));
}

.account-detail__hero {
  display: flex;
  align-items: center;
  gap: var(--space-lg);
  min-width: 0;
  --swatch-ring: var(--bg-base);
}

.account-detail__identity {
  display: grid;
  gap: var(--space-2xs);
  min-width: 0;
}

.account-detail__identity h3,
.account-detail__identity p {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.account-detail__identity p {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.account-detail__notices {
  display: grid;
  gap: var(--space-sm);
}

.account-detail__section {
  display: grid;
  gap: var(--space-sm);
}

.account-detail__hint {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

/* Blocks that bring their own title sit in a card with row padding. */
.account-detail__block,
.setting-group > .account-detail__block {
  padding: var(--space-3-5) var(--space-lg);
  --swatch-ring: var(--bg-surface);
}

.setting-group > * + .account-detail__block,
.setting-group > * + .account-detail__form,
.setting-group > .account-detail__form + .setting-row {
  border-top: 1px solid var(--border-color);
}

.account-detail__actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-xs);
}

.account-detail__busy {
  color: var(--accent-primary);
}

.account-detail__error {
  color: var(--accent-red-soft);
}

.account-detail__form {
  display: grid;
  gap: var(--space-md);
  padding: var(--space-3-5) var(--space-lg);
  background: var(--bg-raised);
}

.account-detail__form-title,
.account-detail__form-copy {
  margin: 0;
}

.account-detail__form-title {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}

.account-detail__form-copy {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.field-row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-2-5);
}

.field {
  display: grid;
  gap: var(--space-xs);
  min-width: 0;
}

.field-label {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-medium);
}

.field input {
  padding: var(--space-sm) var(--space-2-5);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
}

.field input:focus {
  border-color: var(--accent-primary);
}

.form-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
}

.status-msg {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.status-msg--error {
  color: var(--accent-red-soft);
  font-weight: var(--weight-semibold);
}

.status-detail {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.status-detail summary {
  cursor: pointer;
}

.status-detail pre {
  margin: var(--space-xs) 0 0;
  padding: var(--space-sm);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  overflow-x: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

@media (max-width: 767px) {
  .field-row {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
