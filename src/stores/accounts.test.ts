import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendCredentialsStatus,
  backendGetSourceCapabilities,
  backendListAccounts,
  backendCommandsAvailable,
  backendStartOauthFlow,
  backendPollOauthFlow,
} from '@/services/backend';
import { useAccountsStore } from './accounts';
import { useConnectionStore } from './connection';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => false),
  backendClearAccountOauthError: vi.fn(),
  backendCredentialsStatus: vi.fn(),
  backendListAccounts: vi.fn(),
  backendPollOauthFlow: vi.fn(),
  backendStartOauthFlow: vi.fn(),
  backendGetSourceCapabilities: vi.fn(),
  translateBackendError: vi.fn(() => null),
}));

const listAccounts = vi.mocked(backendListAccounts);
const getCapabilities = vi.mocked(backendGetSourceCapabilities);

describe('accounts capabilities', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    listAccounts.mockResolvedValue([
      {
        id: 'mail', display_name: 'Mail', email: 'mail@example.com', provider: 'imap', kind: 'email',
        color: '#111111', last_oauth_error: null, last_oauth_error_at: null, last_sync_at: null,
        sync_status: null, last_sync_error: null, last_sync_error_at: null, sync_progress_json: null,
      },
      {
        id: 'feed', display_name: 'Feed', email: 'https://example.com/feed', provider: 'rss', kind: 'rss',
        color: '#222222', last_oauth_error: null, last_oauth_error_at: null, last_sync_at: null,
        sync_status: null, last_sync_error: null, last_sync_error_at: null, sync_progress_json: null,
      },
    ] as never);
    getCapabilities.mockResolvedValue({
      email: {
        can_reply: true, can_forward: true, can_compose: true, has_folders: true,
        remote_flags: true, remote_archive: true, remote_delete: true,
        has_recipients: true, default_notify: true,
      },
      rss: {
        can_reply: false, can_forward: false, can_compose: false, has_folders: false,
        remote_flags: false, remote_archive: false, remote_delete: false,
        has_recipients: false, default_notify: false,
      },
    });
  });

  it('derives mail-only account lists and actions from backend capabilities', async () => {
    const accounts = useAccountsStore();
    await accounts.load();

    expect(accounts.emailAccounts.map((account) => account.id)).toEqual(['mail']);
    expect(accounts.capabilitiesFor(accounts.accounts[1]).can_reply).toBe(false);
    expect(accounts.capabilitiesFor(accounts.accounts[0]).can_compose).toBe(true);
  });

  it('treats an empty account id as the all-accounts scope', async () => {
    const accounts = useAccountsStore();

    expect(accounts.activeAccountId).toBe('');
    expect(accounts.accountScopeLabel).toBe('All accounts');

    await accounts.load();
    accounts.setActiveAccount('mail');
    expect(accounts.accountScopeLabel).toBe('mail@example.com');

    accounts.setActiveAccount('');
    expect(accounts.accountScopeLabel).toBe('All accounts');
  });

  it('uses safe capability fallbacks if capability loading fails', async () => {
    getCapabilities.mockRejectedValue(new Error('old backend'));
    const accounts = useAccountsStore();
    await accounts.load();

    expect(accounts.capabilitiesFor('email').can_compose).toBe(true);
    expect(accounts.capabilitiesFor('rss').can_compose).toBe(false);
  });
});

describe('account refresh', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.mocked(backendCommandsAvailable).mockReturnValue(true);
    vi.mocked(backendCredentialsStatus).mockResolvedValue({
      total_accounts: 2, encrypted_accounts: 2, plaintext_accounts: 0,
      master_key_present: true, database_bytes: 0, restore_pending: false,
    });
    listAccounts.mockResolvedValue([{ id: 'mail', display_name: 'Mail', email: 'mail@example.com', provider: 'imap' }, { id: 'feed', display_name: 'Feed', email: 'feed@example.com', provider: 'rss' }] as never);
    getCapabilities.mockResolvedValue({});
  });
  it('keeps accounts and the active filter on a transient error', async () => {
    const store = useAccountsStore();
    await store.load();
    store.setActiveAccount('mail');
    listAccounts.mockRejectedValueOnce(new Error('offline'));
    await store.load();
    expect(store.accounts.map(account => account.id)).toEqual(['mail', 'feed']);
    expect(store.activeAccountId).toBe('mail');
    expect(useConnectionStore().backendErrorFor('accounts')).toBe('offline');
  });

  it('ignores an older account response after a newer load', async () => {
    let finish!: (value: never[]) => void;
    listAccounts.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const store = useAccountsStore();
    const first = store.load();
    await store.load();
    finish([]);
    await first;
    expect(store.accounts).toHaveLength(2);
  });
});

describe('credential lock', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.mocked(backendCommandsAvailable).mockReturnValue(true);
  });

  const status = (overrides: Record<string, unknown>) => ({
    total_accounts: 2,
    encrypted_accounts: 2,
    plaintext_accounts: 0,
    master_key_present: true,
    database_bytes: 0,
    restore_pending: false,
    ...overrides,
  });

  it('is locked when the key is out of reach', async () => {
    vi.mocked(backendCredentialsStatus).mockResolvedValue(status({ master_key_present: false }));
    const store = useAccountsStore();

    await store.loadCredentialLock();

    expect(store.credentialLock).toEqual({ keyUnavailable: true, unreadableAccounts: 0 });
  });

  it('counts rows Satchel did not encrypt', async () => {
    vi.mocked(backendCredentialsStatus).mockResolvedValue(
      status({ encrypted_accounts: 1, plaintext_accounts: 1 }),
    );
    const store = useAccountsStore();

    await store.loadCredentialLock();

    expect(store.credentialLock).toEqual({ keyUnavailable: false, unreadableAccounts: 1 });
  });

  it('says nothing when every credential can be used', async () => {
    vi.mocked(backendCredentialsStatus).mockResolvedValue(status({}));
    const store = useAccountsStore();

    await store.loadCredentialLock();

    expect(store.credentialLock).toBeNull();
  });
});

describe('OAuth recovery', () => {
  it.each(['gmail', 'other'])('retries sync and removes only the recovered account failure (%s)', async (recoveredAccountId) => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
    vi.stubGlobal('window', { setTimeout: globalThis.setTimeout });
    try {
      vi.mocked(backendCommandsAvailable).mockReturnValue(true);
      listAccounts.mockResolvedValue([{ id: 'gmail', email: 'mail@example.test', provider: 'gmail' }] as never);
      getCapabilities.mockResolvedValue({});
      vi.mocked(backendStartOauthFlow).mockResolvedValue({ state: 'flow', browser_opened: true } as never);
      vi.mocked(backendPollOauthFlow).mockResolvedValue({ status: 'completed', result: { account_id: recoveredAccountId } } as never);
      const accounts = useAccountsStore();
      const connection = useConnectionStore();
      await accounts.load();
      const failure = (accountId: string) => JSON.stringify({
        key: 'backend.errors.accountSyncFailed', params: { accountId, account: accountId, message: 'old credentials' },
      });
      connection.reportSyncFailures([failure('gmail'), failure('other')]);
      const reconnect = accounts.startReauth('gmail');
      await vi.advanceTimersByTimeAsync(1000);
      await reconnect;
      expect(connection.syncFailures).toEqual([failure(recoveredAccountId === 'gmail' ? 'other' : 'gmail')]);
      expect(connection.retryToken).toBe(1);
    } finally {
      vi.unstubAllGlobals();
      vi.useRealTimers();
    }
  });
});
