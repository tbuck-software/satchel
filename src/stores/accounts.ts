import { computed, ref } from 'vue';
import { defineStore } from 'pinia';
import type { Account } from '@/types/mail';
import {
  backendCommandsAvailable,
  backendClearAccountOauthError,
  backendCredentialsStatus,
  backendGetSourceCapabilities,
  backendListAccounts,
  backendPollOauthFlow,
  backendStartOauthFlow,
  translateBackendError,
  type SourceCapabilities,
} from '@/services/backend';
import { openOAuthAuthorizationUrl } from '@/utils/mobileOAuth';
import { useConnectionStore } from '@/stores/connection';
import { t } from '@/i18n/runtime';

function normalizeProvider(input: string): Account['provider'] {
  return input || 'imap';
}

export const useAccountsStore = defineStore('accounts', () => {
  const connectionStore = useConnectionStore();
  const accounts = ref<Account[]>([]);
  const activeAccountId = ref('');
  const capabilities = ref<Record<string, SourceCapabilities>>({});

  const fallbackCapabilities: SourceCapabilities = {
    can_reply: false,
    can_forward: false,
    can_compose: false,
    has_folders: false,
    remote_flags: false,
    remote_archive: false,
    remote_delete: false,
    has_recipients: false,
    default_notify: false,
  };

  const activeAccount = computed(() =>
    accounts.value.find((account) => account.id === activeAccountId.value),
  );
  // An empty id is the deliberate cross-account scope. Keep that meaning
  // available before the asynchronous account list has arrived as well, so
  // startup never describes the aggregate Inbox as an invalid selection.
  const accountScopeLabel = computed(() => activeAccount.value?.email ?? t('folders.allAccounts'));
  const emailAccounts = computed(() =>
    accounts.value.filter((account) => capabilitiesFor(account).can_compose),
  );

  function capabilitiesFor(accountOrKind?: Account | Account['kind'] | null): SourceCapabilities {
    const kind = typeof accountOrKind === 'string' ? accountOrKind : accountOrKind?.kind;
    // POP3 accounts are kind "email" but the protocol has no folders or
    // server-side flags, so the provider wins when it has its own profile.
    const provider = typeof accountOrKind === 'string' ? undefined : accountOrKind?.provider;
    if (provider && capabilities.value[provider]) {
      return capabilities.value[provider];
    }
    return (kind && capabilities.value[kind]) || {
      ...fallbackCapabilities,
      ...(kind === 'email'
        ? {
            can_reply: true,
            can_forward: true,
            can_compose: true,
            has_folders: true,
            remote_flags: true,
            remote_archive: true,
            remote_delete: true,
            has_recipients: true,
            default_notify: true,
          }
        : {}),
    };
  }

  function setActiveAccount(accountId: string) {
    if (!accountId || accounts.value.some((account) => account.id === accountId)) {
      activeAccountId.value = accountId;
    }
  }

  // Identity palette, same rationale as services/senderAvatar.ts.
  const ACCENT_COLORS = ['#5196ff', '#4cd68a', '#f0b232', '#ff5a5a', '#a97fff', '#3ad4e5', '#f06898', '#ff9f43'];

  function addAccount(input: { displayName: string; email: string; provider: Account['provider'] }): Account {
    const id = `acc-${Date.now().toString(36)}`;
    const colorIndex = accounts.value.length % ACCENT_COLORS.length;
    const account: Account = {
      id,
      displayName: input.displayName || input.email.split('@')[0]!,
      email: input.email,
      provider: input.provider,
      kind: 'email',
      // colorIndex is taken modulo the palette length.
      color: ACCENT_COLORS[colorIndex] ?? 'blue',
    };
    accounts.value = [...accounts.value, account];
    activeAccountId.value = id;
    return account;
  }

  /**
   * Why stored credentials cannot be used: the key is out of reach, or rows
   * hold values Satchel did not encrypt. Either way the accounts stay locked,
   * and a log line nobody reads is not how the user should find out.
   */
  const credentialLock = ref<{ keyUnavailable: boolean; unreadableAccounts: number } | null>(null);

  async function loadCredentialLock() {
    if (!backendCommandsAvailable()) return;
    try {
      const status = await backendCredentialsStatus();
      credentialLock.value = !status.master_key_present || status.plaintext_accounts > 0
        ? { keyUnavailable: !status.master_key_present, unreadableAccounts: status.plaintext_accounts }
        : null;
    } catch {
      credentialLock.value = null;
    }
  }

  let loadRequestId = 0;

  async function load() {
    const requestId = ++loadRequestId;
    void loadCredentialLock();
    try {
      const rows = await backendListAccounts();
      const nextCapabilities = await backendGetSourceCapabilities().catch(() => null);
      if (requestId !== loadRequestId) return;
      if (nextCapabilities) capabilities.value = nextCapabilities;
      accounts.value = rows.map((row) => ({
        id: row.id,
        displayName: row.display_name,
        email: row.email,
        provider: normalizeProvider(row.provider),
        kind: row.kind ?? 'email',
        color: row.color || '#55a6ff',
        lastOauthError: row.last_oauth_error ?? null,
        lastOauthErrorAt: row.last_oauth_error_at ?? null,
        lastSyncAt: row.last_sync_at ?? null,
        syncStatus: row.sync_status ?? null,
        lastSyncError: row.last_sync_error ?? null,
        lastSyncErrorAt: row.last_sync_error_at ?? null,
        syncProgressJson: row.sync_progress_json ?? null,
      }));
      connectionStore.clearBackendError('accounts');

      if (rows.length === 0) {
        activeAccountId.value = '';
        return;
      }

      if (!rows.some((account) => account.id === activeAccountId.value)) {
        activeAccountId.value = rows.length > 1 ? '' : (rows[0]?.id ?? '');
      }
    } catch (error) {
      if (requestId !== loadRequestId) return;
      if (backendCommandsAvailable()) {
        connectionStore.reportBackendError('accounts', error);
      }
    }
  }

  const accountsWithAuthError = computed(() =>
    accounts.value.filter((a) => a.lastOauthError && a.lastOauthError.trim().length > 0),
  );

  async function startReauth(accountId: string, purpose?: 'contacts' | 'calendar') {
    const account = accounts.value.find((a) => a.id === accountId);
    if (!account) return;
    if (account.provider !== 'gmail' && account.provider !== 'outlook') {
      return;
    }
    const flow = await backendStartOauthFlow({ accountId, provider: account.provider, ...(purpose ? { purpose } : {}) });
    // A phone has no desktop session for the Rust opener; open it from the shell.
    if (!flow.browser_opened) await openOAuthAuthorizationUrl(flow.authorization_url);
    const recoveredAccountId = await waitForOAuthCompletion(flow.state);
    if (recoveredAccountId) connectionStore.clearAccountSyncFailure(recoveredAccountId);
    connectionStore.requestRetry();
    await load();
  }

  async function waitForOAuthCompletion(state: string) {
    const deadline = Date.now() + 2 * 60_000;
    while (Date.now() < deadline) {
      await new Promise((resolve) => {
        window.setTimeout(resolve, 1_000);
      });
      const result = await backendPollOauthFlow(state);
      if (result.status === 'completed') {
        return result.result?.account_id;
      }
      if (result.status === 'failed') {
        throw new Error(t('settings.accounts.reauth.failed', {
          error: translateBackendError(result.error) || t('errors.backendUnknown'),
        }));
      }
    }
    throw new Error(t('settings.accounts.reconnect.timeout'));
  }

  async function clearAuthError(accountId: string) {
    try {
      await backendClearAccountOauthError(accountId);
    } catch {}
    const idx = accounts.value.findIndex((a) => a.id === accountId);
    const existing = idx >= 0 ? accounts.value[idx] : undefined;
    if (idx >= 0 && existing) {
      accounts.value[idx] = {
        ...existing,
        lastOauthError: null,
        lastOauthErrorAt: null,
      };
    }
  }

  return {
    accounts,
    activeAccountId,
    activeAccount,
    accountScopeLabel,
    emailAccounts,
    capabilities,
    capabilitiesFor,
    accountsWithAuthError,
    credentialLock,
    loadCredentialLock,
    setActiveAccount,
    addAccount,
    load,
    startReauth,
    clearAuthError,
  };
});
