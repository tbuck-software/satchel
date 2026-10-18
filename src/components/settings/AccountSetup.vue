<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRoute } from 'vue-router';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useThreadsStore } from '@/stores/threads';
import { useUiStore } from '@/stores/ui';
import {
  backendAutoconfigureGroupware,
  backendDeleteAccount,
  backendSupportsNativeCommands,
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
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';

const { locale, t } = useI18n();
const accounts = useAccountsStore();
const connection = useConnectionStore();
const mailboxes = useMailboxesStore();
const threads = useThreadsStore();
const ui = useUiStore();
const { ask } = useConfirm();
const oauthAvailable = backendSupportsNativeCommands();
const emailAccounts = computed(() => accounts.emailAccounts);

const syncBusy = ref(false);
const statusText = ref('');
/**
 * The untouched chain behind the last failure, paired with the line it belongs
 * to. The pairing is what keeps every success path untouched: a newer status
 * no longer matches, so stale details cannot outlive the failure.
 */
const lastFailure = ref({ status: '', raw: '' });
const statusDetail = computed(() =>
  lastFailure.value.status === statusText.value ? lastFailure.value.raw : '',
);
const groupwareOpen = ref(false);
const groupwareBusy = ref(false);
const groupwareAccountId = ref('');
const groupwareAccountEmail = ref('');
const groupwareUsername = ref('');
const groupwarePassword = ref('');
const caldavServer = ref('');
const carddavServer = ref('');
const route = useRoute();
const passwordAccountId = ref('');
const newPassword = ref('');
const passwordBusy = ref(false);
const passwordInput = ref<HTMLInputElement | null>(null);
const passwordAccount = computed(() =>
  emailAccounts.value.find((account) => account.id === passwordAccountId.value) ?? null,
);

/** Put the class of failure in the status line and the chain behind it. */
function reportFailure(key: string, error: unknown) {
  const raw = transportErrorText(error);
  const label = transportErrorLabel(error);
  statusText.value = t(key, { error: label || raw });
  lastFailure.value = { status: statusText.value, raw: label === raw ? '' : raw };
}

function providerLabel(provider: string): string {
  if (provider === 'gmail') return 'Gmail';
  if (provider === 'outlook') return 'Outlook';
  if (provider === 'pop3') return 'POP3';
  return 'IMAP';
}

function syncStatusLabel(account: (typeof accounts.accounts)[number]): string {
  const status = account.syncStatus ?? 'idle';
  if (status === 'syncing') return t('settings.accounts.sync.running');
  if (status === 'error') return t('settings.accounts.sync.error');
  if (account.lastSyncAt) {
    return t('settings.accounts.sync.lastAt', {
      time: new Date(account.lastSyncAt).toLocaleString(locale.value),
    });
  }
  return t('settings.accounts.sync.never');
}

function syncProgressLabel(raw?: string | null): string {
  if (!raw) return '';
  try {
    const parsed = JSON.parse(raw) as {
      phase?: string;
      folder?: string;
      folder_index?: number;
      folder_total?: number;
    };
    if (parsed.phase === 'folder' && parsed.folder) {
      return `${parsed.folder_index ?? '?'} / ${parsed.folder_total ?? '?'}: ${parsed.folder}`;
    }
    if (parsed.phase === 'complete') return t('settings.accounts.sync.progressComplete');
    if (parsed.phase === 'error') return t('common.error');
  } catch {}
  return '';
}

function statusButtonTitle(provider: string): string {
  if (provider === 'imap') {
    return t('settings.accounts.testImap');
  }
  if (provider === 'pop3') {
    return t('settings.accounts.testPop3');
  }
  return t('settings.accounts.checkOrRenew');
}

function groupwareErrorSuffix(errors: string[]): string {
  if (errors.length === 0) return '';
  return `; ${errors.map((error) => translateBackendError(error) ?? error).join('; ')}`;
}

function openGroupware(account: (typeof accounts.accounts)[number]) {
  if (groupwareOpen.value && groupwareAccountId.value === account.id) {
    groupwareOpen.value = false;
    return;
  }
  const domain = account.email.split('@')[1]?.trim() ?? '';
  groupwareAccountId.value = account.id;
  groupwareAccountEmail.value = account.email;
  groupwareUsername.value = account.email.split('@')[0]?.trim() ?? account.email;
  groupwarePassword.value = '';
  caldavServer.value = domain ? `gwcaldav.${domain}` : '';
  carddavServer.value = domain ? `gwcarddav.${domain}` : '';
  groupwareOpen.value = true;
}

async function connectGroupware(account: (typeof accounts.accounts)[number]) {
  groupwareAccountId.value = account.id;
  groupwareAccountEmail.value = account.email;
  groupwareBusy.value = true;
  statusText.value = t('settings.accounts.groupware.discovering');
  try {
    const result = await backendAutoconfigureGroupware(account.id);
    let services = t('settings.accounts.groupware.contacts');
    if (result.caldav_server && result.carddav_server) {
      services = t('settings.accounts.groupware.calendarAndContacts');
    } else if (result.caldav_server) {
      services = t('settings.accounts.groupware.calendar');
    }
    const suffix = groupwareErrorSuffix(result.summary.errors);
    statusText.value = t('settings.accounts.groupware.connected', {
      services,
      events: result.summary.events_synced,
      contacts: result.summary.contacts_synced,
      suffix,
    });
    groupwareOpen.value = false;
  } catch (error) {
    reportFailure('settings.accounts.groupware.autoFailed', error);
    groupwareOpen.value = false;
    openGroupware(account);
  } finally {
    groupwareBusy.value = false;
  }
}

async function saveGroupware() {
  if (!groupwareAccountId.value || !groupwareUsername.value.trim() || !groupwarePassword.value) {
    statusText.value = t('settings.accounts.groupware.credentialsRequired');
    return;
  }
  if (!caldavServer.value.trim() && !carddavServer.value.trim()) {
    statusText.value = t('settings.accounts.groupware.serverRequired');
    return;
  }
  groupwareBusy.value = true;
  try {
    const result = await backendConfigureGroupware({
      accountId: groupwareAccountId.value,
      username: groupwareUsername.value.trim(),
      password: groupwarePassword.value,
      caldavServer: caldavServer.value.trim() || null,
      carddavServer: carddavServer.value.trim() || null,
    });
    const suffix = groupwareErrorSuffix(result.errors);
    statusText.value = t('settings.accounts.groupware.saved', {
      events: result.events_synced,
      contacts: result.contacts_synced,
      suffix,
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
  if (!groupwareAccountId.value) return;
  groupwareBusy.value = true;
  try {
    const result = await backendSyncGroupware(groupwareAccountId.value);
    const suffix = groupwareErrorSuffix(result.errors);
    statusText.value = t('settings.accounts.groupware.synced', {
      events: result.events_synced,
      contacts: result.contacts_synced,
      suffix,
    });
  } catch (error) {
    reportFailure('settings.accounts.groupware.syncFailed', error);
  } finally {
    groupwareBusy.value = false;
  }
}

function usesPassword(provider: string): boolean {
  return provider === 'imap' || provider === 'pop3';
}

function openPasswordForm(accountId: string) {
  passwordAccountId.value = accountId;
  newPassword.value = '';
  // Focusing scrolls the field into view as well.
  void nextTick(() => passwordInput.value?.focus());
}

function togglePasswordForm(accountId: string) {
  if (passwordAccountId.value === accountId) passwordAccountId.value = '';
  else openPasswordForm(accountId);
}

// `?account=` is how the connection banner's "Update password" lands here.
watch(
  () => [route?.query.account, emailAccounts.value.length] as const,
  ([requested]) => {
    if (typeof requested !== 'string' || passwordAccountId.value === requested) return;
    const account = emailAccounts.value.find((candidate) => candidate.id === requested);
    if (account && usesPassword(account.provider)) openPasswordForm(account.id);
  },
  { immediate: true },
);

async function savePassword() {
  const account = passwordAccount.value;
  if (!account || !newPassword.value || passwordBusy.value) return;
  passwordBusy.value = true;
  try {
    await backendUpdateAccountPassword(account.id, newPassword.value);
    newPassword.value = '';
    passwordAccountId.value = '';
    statusText.value = t('settings.accounts.password.saved', { email: account.email });
    connection.requestRetry();
    void accounts.load();
  } catch (error) {
    reportFailure('settings.accounts.password.failed', error);
  } finally {
    passwordBusy.value = false;
  }
}

async function syncImapNow(showIfEmpty = true) {
  if (!oauthAvailable) {
    statusText.value = t('settings.accounts.sync.requiresApp');
    return;
  }

  syncBusy.value = true;
  try {
    const summary = await backendRunImapSync();
    // Same numbers this pane already prints, for the inbox that has no reply
    // of its own to read them from.
    connection.applySyncSummary(summary);
    await Promise.all([threads.load(), mailboxes.load(), accounts.load()]);
    const total = summary.messages_synced;
    const activeAccountId = accounts.activeAccountId;
    const localThreadCount = threads.threads.filter((thread) =>
      activeAccountId ? thread.accountId === activeAccountId : true,
    ).length;
    const backfillTotal = Math.max(summary.backfill_total_messages, 0);
    const backfillRemaining = Math.max(summary.backfill_remaining_messages, 0);
    const backfillDone = Math.max(backfillTotal - backfillRemaining, 0);
    const backfillPercent =
      backfillTotal > 0 ? Math.min(100, Math.round((backfillDone / backfillTotal) * 100)) : 0;
    const backfillProgress =
      backfillTotal > 0
        ? t('settings.accounts.sync.backfillProgress', {
            done: backfillDone,
            total: backfillTotal,
            percent: backfillPercent,
          })
        : t('settings.accounts.sync.backfillProgressPlain', { done: backfillDone });
    const threadsCount = t(
      'settings.accounts.sync.threadsCount',
      { count: localThreadCount },
      localThreadCount,
    );
    if (summary.failures.length > 0 || summary.accounts_failed > 0) {
      const rawFailure = summary.failures[0];
      const firstFailure = rawFailure
        ? translateBackendError(rawFailure) ?? rawFailure
        : t('settings.accounts.sync.unknownFailure');
      const failedCount = Math.max(summary.accounts_failed, summary.failures.length);
      statusText.value = t('settings.accounts.sync.partial', {
        messages: t(
          'settings.accounts.sync.messagesCount',
          { count: summary.messages_synced },
          summary.messages_synced,
        ),
        failed: failedCount,
        error: firstFailure,
        threads: threadsCount,
      });
    } else if (total === 0 && showIfEmpty) {
      if (summary.backfill_pending_accounts > 0) {
        statusText.value = t('settings.accounts.sync.backfillStatus', {
          progress: backfillProgress,
          remaining: backfillRemaining,
          accounts: t(
            'settings.accounts.sync.accountsCount',
            { count: summary.backfill_pending_accounts },
            summary.backfill_pending_accounts,
          ),
          threads: threadsCount,
        });
      } else {
        statusText.value = t('settings.accounts.sync.finishedEmpty', { threads: threadsCount });
      }
    } else {
      const messagesCount = t(
        'settings.accounts.sync.messagesCount',
        { count: summary.messages_synced },
        summary.messages_synced,
      );
      const accountsCount = t(
        'settings.accounts.sync.accountsCount',
        { count: summary.accounts_synced },
        summary.accounts_synced,
      );
      if (summary.backfill_pending_accounts > 0) {
        statusText.value = t('settings.accounts.sync.finishedWithBackfill', {
          messages: messagesCount,
          accounts: accountsCount,
          progress: backfillProgress,
          remaining: backfillRemaining,
          pending: t(
            'settings.accounts.sync.accountsCount',
            { count: summary.backfill_pending_accounts },
            summary.backfill_pending_accounts,
          ),
          threads: threadsCount,
        });
      } else {
        statusText.value = t('settings.accounts.sync.finished', {
          messages: messagesCount,
          accounts: accountsCount,
          threads: threadsCount,
        });
      }
    }
  } catch (error) {
    reportFailure('settings.accounts.sync.failed', error);
  } finally {
    syncBusy.value = false;
  }
}

async function checkStatus(accountId: string, provider: string) {
  if (provider === 'imap') {
    try {
      const result = await backendTestImapAccountConnection(accountId);
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
    const result = await backendOauthStatus(accountId);
    statusText.value = t('settings.accounts.oauthStatus', {
      provider: result.provider,
      state: result.connected
        ? t('settings.accounts.connected')
        : t('settings.accounts.disconnected'),
    });
  } catch (error) {
    reportFailure('settings.accounts.statusUnavailable', error);
  }
}

const reauthBusyId = ref('');

async function reconnectOauth(account: (typeof accounts.accounts)[number]) {
  if (!oauthAvailable || reauthBusyId.value) return;
  reauthBusyId.value = account.id;
  statusText.value = t('settings.accounts.reconnect.browserOpened', { email: account.email });
  try {
    await accounts.startReauth(account.id);
    statusText.value = t('settings.accounts.reconnect.done', { email: account.email });
  } catch (error) {
    reportFailure('settings.accounts.reconnect.failed', error);
  } finally {
    reauthBusyId.value = '';
  }
}

async function removeAccount(accountId: string, email: string) {
  if (!oauthAvailable) {
    statusText.value = t('settings.accounts.removeRequiresApp');
    return;
  }

  // A trash icon with no dialog and no undo, next to twelve other icons — and
  // it used to only deactivate, so "removed" was a word for hiding gigabytes.
  const confirmed = await ask({
    title: t('settings.accounts.removeConfirmTitle'),
    body: t('settings.accounts.removeConfirmBody'),
    detail: email,
    confirmLabel: t('settings.accounts.removeConfirmAction'),
    tone: 'danger',
  });
  if (!confirmed) return;

  try {
    await backendDeleteAccount(accountId);
    await accounts.load();
    await Promise.all([threads.load(), mailboxes.load()]);
    statusText.value = t('settings.accounts.removed', { email });
  } catch (error) {
    reportFailure('settings.accounts.removeFailed', error);
  }
}
</script>

<template>
  <div class="section">
    <header class="section-header">
      <h3>{{ $t('settings.sections.accounts.label') }}</h3>
      <p class="section-desc">{{ $t('settings.accounts.description') }}</p>
    </header>

    <!-- Account Cards -->
    <div class="account-list">
      <article v-for="account in emailAccounts" :key="account.id" class="account-card">
        <span class="account-color" :style="{ backgroundColor: account.color }" />
        <div class="account-info">
          <span class="account-name">{{ account.displayName }}</span>
          <span class="account-email">{{ account.email }}</span>
          <span
            class="account-sync"
            :class="{ error: account.syncStatus === 'error', busy: account.syncStatus === 'syncing' }"
          >
            {{ syncStatusLabel(account) }}
            <template v-if="syncProgressLabel(account.syncProgressJson)">
              · {{ syncProgressLabel(account.syncProgressJson) }}
            </template>
            <template v-if="account.lastSyncError">
              · {{ translateBackendError(account.lastSyncError) ?? account.lastSyncError }}
            </template>
          </span>
        </div>
        <span class="account-provider">{{ providerLabel(account.provider) }}</span>
        <IconButton
          v-if="account.provider === 'imap'"
          :title="$t('settings.accounts.groupware.connectTitle')"
          :disabled="!oauthAvailable || groupwareBusy"
          @click.stop.prevent="connectGroupware(account)"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="3.5" width="12" height="10.5" rx="1.5" />
            <path d="M5 1.5v4M11 1.5v4M2 7h12" />
          </svg>
        </IconButton>
        <IconButton
          v-if="usesPassword(account.provider)"
          :title="$t('settings.accounts.password.action')"
          :active="passwordAccountId === account.id"
          :disabled="!oauthAvailable"
          @click.stop.prevent="togglePasswordForm(account.id)"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="5.5" cy="10.5" r="3" />
            <path d="m7.6 8.4 6-6M11.5 4.5l2 2M10 6l1.5 1.5" />
          </svg>
        </IconButton>
        <IconButton
          v-if="account.provider === 'gmail' || account.provider === 'outlook'"
          :title="$t('settings.accounts.reconnect.buttonTitle')"
          :disabled="!oauthAvailable || reauthBusyId === account.id"
          @click.stop.prevent="reconnectOauth(account)"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M13.5 8A5.5 5.5 0 1 1 11.9 4.1" />
            <path d="M13.5 1.5v3h-3" />
          </svg>
        </IconButton>
        <IconButton
          :title="statusButtonTitle(account.provider)"
          :disabled="!oauthAvailable"
          @click.stop.prevent="checkStatus(account.id, account.provider)"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path v-if="account.provider === 'imap'" d="M6 1.5v3M10 1.5v3" />
            <path
              v-if="account.provider === 'imap'"
              d="M4 6h8a1 1 0 011 1v2a3 3 0 01-3 3H6a3 3 0 01-3-3V7a1 1 0 011-1z"
            />
            <path v-if="account.provider === 'imap'" d="M8 12.5V15" />
            <path v-if="account.provider !== 'imap'" d="M1 8a7 7 0 0114 0 7 7 0 01-14 0z" />
            <path v-if="account.provider !== 'imap'" d="M8 5v3l2 1" />
          </svg>
        </IconButton>
        <IconButton
          tone="danger"
          :title="$t('settings.accounts.removeTitle')"
          :disabled="!oauthAvailable"
          @click.stop.prevent="removeAccount(account.id, account.email)"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M2.5 4h11" />
            <path d="M6 2.5h4" />
            <path d="M4 4l.6 9a1 1 0 001 1h4.8a1 1 0 001-1L12 4" />
          </svg>
        </IconButton>
      </article>
    </div>

    <form v-if="passwordAccount" class="oauth-form password-form" @submit.prevent="savePassword">
      <div class="oauth-hero">
        <p class="oauth-hero-title">
          {{ $t('settings.accounts.password.formTitle', { email: passwordAccount.email }) }}
        </p>
        <p class="oauth-hero-copy">{{ $t('settings.accounts.password.formCopy') }}</p>
      </div>
      <label class="field">
        <span class="field-label">{{ $t('settings.accounts.password.label') }}</span>
        <input
          ref="passwordInput"
          v-model="newPassword"
          type="password"
          autocomplete="current-password"
          :placeholder="$t('settings.accounts.password.placeholder')"
        />
      </label>
      <div class="form-actions">
        <AppButton type="button" :disabled="passwordBusy" @click="passwordAccountId = ''">
          {{ $t('common.cancel') }}
        </AppButton>
        <AppButton type="submit" variant="primary" :disabled="passwordBusy || !newPassword">
          {{
            passwordBusy
              ? $t('settings.accounts.password.checking')
              : $t('settings.accounts.password.save')
          }}
        </AppButton>
      </div>
    </form>

    <div v-if="groupwareOpen" class="oauth-form groupware-form">
      <div class="oauth-hero">
        <div class="oauth-provider-badge">
          <span class="oauth-provider-dot" /><span>GroupWise</span>
        </div>
        <p class="oauth-hero-title">
          {{ $t('settings.accounts.groupware.formTitle', { email: groupwareAccountEmail }) }}
        </p>
        <p class="oauth-hero-copy">{{ $t('settings.accounts.groupware.formCopy') }}</p>
      </div>

      <div class="field-row">
        <label class="field grow">
          <span class="field-label">{{ $t('settings.accounts.groupware.username') }}</span>
          <input
            v-model="groupwareUsername"
            autocomplete="username"
            :placeholder="$t('settings.accounts.groupware.usernamePlaceholder')"
          />
        </label>
        <label class="field grow">
          <span class="field-label">{{ $t('settings.accounts.groupware.password') }}</span>
          <input
            v-model="groupwarePassword"
            type="password"
            autocomplete="current-password"
            :placeholder="$t('settings.accounts.groupware.passwordPlaceholder')"
          />
        </label>
      </div>
      <div class="field-row">
        <label class="field grow">
          <span class="field-label">{{ $t('settings.accounts.groupware.caldavServer') }}</span>
          <input v-model="caldavServer" :placeholder="$t('settings.accounts.groupware.caldavServerPlaceholder')" />
        </label>
        <label class="field grow">
          <span class="field-label">{{ $t('settings.accounts.groupware.carddavServer') }}</span>
          <input v-model="carddavServer" :placeholder="$t('settings.accounts.groupware.carddavServerPlaceholder')" />
        </label>
      </div>
      <div class="form-actions">
        <AppButton :disabled="groupwareBusy" @click="syncGroupwareNow">
          {{ $t('settings.accounts.groupware.syncNow') }}
        </AppButton>
        <AppButton variant="primary" :disabled="groupwareBusy" @click="saveGroupware">
          {{
            groupwareBusy
              ? $t('settings.accounts.groupware.checking')
              : $t('settings.accounts.groupware.connect')
          }}
        </AppButton>
      </div>
    </div>

    <!-- Status message -->
    <p v-if="statusText" class="status-msg">{{ statusText }}</p>
    <details v-if="statusDetail" class="status-detail">
      <summary>{{ $t('errors.technicalDetails') }}</summary>
      <pre>{{ statusDetail }}</pre>
    </details>

    <div class="form-actions">
      <AppButton :disabled="!oauthAvailable || syncBusy" @click="syncImapNow()">
        {{ syncBusy ? $t('settings.accounts.sync.syncing') : $t('settings.accounts.sync.syncImapNow') }}
      </AppButton>
    </div>

    <button type="button" class="add-account-button" @click="ui.openAccountSetup()">
      <span class="add-account-icon">
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M8 3v10M3 8h10" />
        </svg>
      </span>
      <span>{{ $t('settings.accounts.addAccount') }}</span>
      <svg class="add-account-arrow" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 8h10M9 4l4 4-4 4" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.section {
  display: grid;
  gap: 20px;
}

.section-header h3 {
  margin: 0;
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
  color: var(--text-primary);
}

.section-desc {
  margin: 4px 0 0;
  font-size: var(--font-size-sm);
  color: var(--text-tertiary);
}

/* Account Cards */
.account-list {
  display: grid;
  gap: 6px;
}

.account-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  transition: background var(--duration-fast) var(--ease-out);
}

.account-card:hover {
  background: var(--bg-overlay);
}

.account-color {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.account-info {
  display: grid;
  gap: 1px;
  min-width: 0;
  flex: 1;
}

.account-name {
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.account-email {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.account-sync {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.account-sync.busy {
  color: var(--accent-primary);
}

.account-sync.error {
  color: var(--accent-red-soft);
}

.account-provider {
  font-size: var(--font-size-xs);
  font-weight: var(--weight-medium);
  color: var(--text-secondary);
  padding: 3px 8px;
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
  flex-shrink: 0;
}

.status-msg {
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--accent-primary);
  font-family: var(--font-mono);
}

.status-detail {
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
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

.add-account-button {
  display: flex;
  align-items: center;
  gap: 11px;
  width: 100%;
  padding: 11px 13px;
  border: 1px solid color-mix(in srgb, var(--accent-primary) 24%, var(--border-color-muted));
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--accent-primary) 4%, var(--bg-raised));
  font-family: var(--font-sans);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
  cursor: pointer;
  transition: border-color var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
}

.add-account-button:hover {
  border-color: color-mix(in srgb, var(--accent-primary) 48%, var(--border-color));
  color: var(--text-primary);
  background: color-mix(in srgb, var(--accent-primary) 8%, var(--bg-raised));
}

.add-account-icon {
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border-radius: 9px 9px 9px 4px;
  color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 12%, transparent);
}

.add-account-arrow {
  margin-left: auto;
  color: var(--text-tertiary);
  transition: transform var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
}

.add-account-button:hover .add-account-arrow {
  transform: translateX(2px);
  color: var(--accent-primary);
}

/* OAuth Section */
.oauth-section {
  display: grid;
  gap: 0;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  overflow: hidden;
}

.oauth-trigger {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 12px 16px;
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  font-family: var(--font-sans);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.oauth-trigger:hover {
  background: var(--bg-overlay);
  color: var(--text-primary);
}

.oauth-trigger:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.chevron {
  margin-left: auto;
  transition: transform var(--duration-normal) var(--ease-out);
}

.chevron.open {
  transform: rotate(180deg);
}

.oauth-form {
  display: grid;
  gap: 12px;
  padding: 0 16px 16px;
  border-top: 1px solid var(--border-color-muted);
  padding-top: 14px;
}

.oauth-hero {
  display: grid;
  gap: 8px;
  padding: 14px;
  border-radius: var(--radius-md);
  background:
    radial-gradient(circle at top left, color-mix(in srgb, var(--accent-primary) 16%, transparent), transparent 45%),
    var(--bg-surface);
  /* The card sits on --bg-base; the fill alone is a 1.1:1 step, so the border
     is what actually separates it in the light theme. */
  border: 1px solid var(--border-color);
}

.oauth-provider-badge {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  width: fit-content;
  padding: 6px 10px;
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
  text-transform: uppercase;
  letter-spacing: 0.06em;
}

.oauth-provider-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--accent-primary);
}

.oauth-provider-badge[data-provider='outlook'] .oauth-provider-dot {
  background: var(--accent-primary);
}

.oauth-hero-title,
.oauth-hero-copy {
  margin: 0;
}

.oauth-hero-title {
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
}

.oauth-hero-copy {
  color: var(--text-secondary);
  line-height: 1.5;
}

.oauth-hero-copy code {
  font-family: var(--font-mono);
  color: var(--text-primary);
}

.oauth-fallback,
.manual-panel {
  display: grid;
  gap: 10px;
  padding: 12px;
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  border: 1px solid var(--border-color-muted);
}

.oauth-fallback-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.oauth-link-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.text-link {
  color: var(--accent-primary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
  text-decoration: none;
}

.text-link:hover {
  opacity: 0.82;
}

.button-link {
  border: 0;
  padding: 0;
  background: transparent;
  cursor: pointer;
  font-family: var(--font-sans);
}

.manual-toggle {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  border: 0;
  padding: 0;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  font-family: var(--font-sans);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
}

.manual-toggle:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.oauth-waiting {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.waiting-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--accent-primary);
  box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent-primary) 45%, transparent);
  animation: waiting-pulse 1.4s ease-out infinite;
}

.field-row {
  display: flex;
  gap: 10px;
  align-items: flex-end;
}

.field {
  display: grid;
  gap: 5px;
}

.field.grow {
  flex: 1;
  min-width: 0;
}

.field-label {
  font-size: var(--font-size-xs);
  font-weight: var(--weight-medium);
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

input,
select,
textarea {
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-primary);
  padding: 8px 10px;
  font-family: var(--font-sans);
  font-size: var(--font-size-sm);
  transition: border-color var(--duration-fast) var(--ease-out);
}

input:focus,
select:focus,
textarea:focus {
  border-color: var(--accent-primary);
}

textarea {
  resize: vertical;
  font-family: var(--font-mono);
  font-size: var(--font-size-xs);
}

select {
  cursor: pointer;
}

.form-actions {
  display: flex;
  justify-content: flex-end;
}

@keyframes waiting-pulse {
  0% {
    box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent-primary) 40%, transparent);
  }

  70% {
    box-shadow: 0 0 0 10px transparent;
  }

  100% {
    box-shadow: 0 0 0 0 transparent;
  }
}
</style>
