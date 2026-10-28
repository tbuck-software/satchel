import { computed, onScopeDispose, ref } from 'vue';
import { defineStore } from 'pinia';
import type { Account } from '@/types/mail';
import {
  backendCommandsAvailable,
  backendGetAccountVisibility,
  backendSetAccountVisibility,
  type BackendAccountVisibilityPolicy,
  type BackendAccountVisibilityStatus,
  backendClearAccountOauthError,
  backendCredentialsStatus,
  backendGetSourceCapabilities,
  backendListAccounts,
  backendPollOauthFlow,
  backendSetAccountAvatar,
  backendSetAccountColor,
  backendStartOauthFlow,
  translateBackendError,
  type SourceCapabilities,
} from '@/services/backend';
import { useConnectionStore } from '@/stores/connection';
import { t } from '@/i18n/runtime';
import { useToast } from '@/composables/useToast';
import { ACCOUNT_COLORS, DEFAULT_ACCOUNT_COLOR } from '@/config/accountColors';

function normalizeProvider(input: string): Account['provider'] {
  return input || 'imap';
}

export const useAccountsStore = defineStore('accounts', () => {
  const connectionStore = useConnectionStore();
  const accounts = ref<Account[]>([]);
  const activeAccountId = ref('');
  let accountSelectionExplicit = false;
  const capabilities = ref<Record<string, SourceCapabilities>>({});
  const visibility = ref<BackendAccountVisibilityStatus[]>([]);
  const visibilityReady = ref(false);
  const visibilityBusy = ref(false);
  const toast = useToast();
  const restingAccountIds = computed(() => new Set(visibility.value.filter(row => !row.visible).map(row => row.account_id)));
  const visibilityKey = computed(() => [...restingAccountIds.value].sort().join(','));
  function isVisible(accountId: string): boolean {
    return !restingAccountIds.value.has(accountId);
  }
  function visibilityFor(accountId: string): BackendAccountVisibilityStatus | undefined {
    return visibility.value.find(row => row.account_id === accountId);
  }
  /** Shown today against its schedule, until local midnight. */
  function isPeeked(accountId: string): boolean {
    const row = visibilityFor(accountId);
    return Boolean(row && isVisible(accountId) && row.policy.mode !== 'always'
      && row.policy.override_date === row.local_date && row.policy.override_visible === true);
  }
  let visibilityRequest = 0;
  let visibilityTimer: ReturnType<typeof setInterval> | undefined;
  async function refreshVisibility() {
    if (!backendCommandsAvailable()) return false;
    const request = ++visibilityRequest;
    try {
      const next = await backendGetAccountVisibility();
      if (request !== visibilityRequest) return false;
      if (JSON.stringify(next) !== JSON.stringify(visibility.value)) visibility.value = next;
      // A single mailbox is selected automatically at startup. That is not
      // an explicit request to keep looking at it after its window ends.
      if (!accountSelectionExplicit && next.some(row => row.account_id === activeAccountId.value && !row.visible)) {
        activeAccountId.value = '';
      }
      visibilityReady.value = true;
      if (!visibilityTimer) {
        visibilityTimer = setInterval(() => void refreshVisibility(), 15_000);
        if (typeof window !== 'undefined') {
          window.addEventListener('focus', refreshVisibility);
          document.addEventListener('visibilitychange', refreshVisibility);
        }
      }
      return true;
    } catch (error) {
      connectionStore.reportBackendError('accounts', error);
      return false;
    }
  }
  onScopeDispose(() => {
    if (visibilityTimer) clearInterval(visibilityTimer);
    if (typeof window !== 'undefined') {
      window.removeEventListener('focus', refreshVisibility);
      document.removeEventListener('visibilitychange', refreshVisibility);
    }
  });
  async function setVisibility(accountId: string, policy: BackendAccountVisibilityPolicy): Promise<boolean> {
    if (visibilityBusy.value) return false;
    visibilityBusy.value = true;
    ++visibilityRequest;
    try {
      await backendSetAccountVisibility(accountId, policy);
      if (!await refreshVisibility()) {
        toast.error(t('accountVisibility.saveFailed'));
        return false;
      }
      connectionStore.clearBackendError('accounts');
      return true;
    } catch (error) {
      connectionStore.reportBackendError('accounts', error);
      toast.error(t('accountVisibility.saveFailed'));
      return false;
    } finally {
      visibilityBusy.value = false;
    }
  }
  async function showToday(accountId: string, visible: boolean) {
    if (!await refreshVisibility()) return false;
    const row = visibilityFor(accountId);
    if (!row) return;
    return setVisibility(accountId, { ...row.policy, override_date: row.local_date, override_visible: visible });
  }


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
      accountSelectionExplicit = true;
    }
  }

  function addAccount(input: { displayName: string; email: string; provider: Account['provider'] }): Account {
    const id = `acc-${Date.now().toString(36)}`;
    const colorIndex = accounts.value.length % ACCOUNT_COLORS.length;
    const account: Account = {
      id,
      displayName: input.displayName || input.email.split('@')[0]!,
      email: input.email,
      provider: input.provider,
      kind: 'email',
      // colorIndex is taken modulo the palette length.
      color: ACCOUNT_COLORS[colorIndex]?.value ?? DEFAULT_ACCOUNT_COLOR,
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
      const [rows] = await Promise.all([backendListAccounts(), refreshVisibility()]);
      const nextCapabilities = await backendGetSourceCapabilities().catch(() => null);
      if (requestId !== loadRequestId) return;
      if (nextCapabilities) capabilities.value = nextCapabilities;
      accounts.value = rows.map((row) => ({
        id: row.id,
        displayName: row.display_name,
        email: row.email,
        provider: normalizeProvider(row.provider),
        kind: row.kind ?? 'email',
        color: row.color || DEFAULT_ACCOUNT_COLOR,
        avatar: row.avatar ?? null,
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
        if (!activeAccountId.value && accountSelectionExplicit) return;
        accountSelectionExplicit = false;
        const soleAccount = rows.length === 1 ? rows[0] : undefined;
        activeAccountId.value = soleAccount && isVisible(soleAccount.id) ? soleAccount.id : '';
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
    if (!flow.browser_opened) {
      const { openOAuthAuthorizationUrl } = await import('@/utils/mobileOAuth');
      await openOAuthAuthorizationUrl(flow.authorization_url);
    }
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

  /**
   * Change how an account looks: shown at once, written behind it, put back
   * with a toast if the backend refuses.
   */
  async function updateAppearance(accountId: string, change: { color: string } | { avatar: string | null }) {
    const index = accounts.value.findIndex((account) => account.id === accountId);
    const before = accounts.value[index];
    if (!before) return false;
    accounts.value[index] = { ...before, ...change };
    try {
      if ('color' in change) await backendSetAccountColor(accountId, change.color);
      else await backendSetAccountAvatar(accountId, change.avatar);
      return true;
    } catch (error) {
      const current = accounts.value.findIndex((account) => account.id === accountId);
      if (current >= 0) accounts.value[current] = before;
      toast.error(translateBackendError(error) || t('accountAppearance.saveFailed'));
      return false;
    }
  }

  function setColor(accountId: string, color: string) {
    return updateAppearance(accountId, { color });
  }

  function setAvatar(accountId: string, avatar: string | null) {
    return updateAppearance(accountId, { avatar });
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
    visibility,
    visibilityReady,
    visibilityBusy,
    visibilityKey,
    isVisible,
    isPeeked,
    visibilityFor,
    refreshVisibility,
    setVisibility,
    showToday,
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
    setColor,
    setAvatar,
  };
});
