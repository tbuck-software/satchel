// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  backendSyncGoogleCalendar: vi.fn(),
}));
vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  backendCommandsAvailable: () => true,
  ...backend,
}));

const route = vi.hoisted(() => ({ query: {} as Record<string, string> }));
const router = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));
vi.mock('vue-router', () => ({ useRoute: () => route, useRouter: () => router }));

const accounts = vi.hoisted(() => {
  const gmail = {
    id: 'gmail-account',
    displayName: 'Private',
    label: 'Private' as string | null,
    senderName: 'Tom Example',
    email: 'person@example.test',
    provider: 'gmail',
    kind: 'email',
    color: '#123456',
    syncStatus: 'idle',
  };
  const imap = {
    id: 'imap-account',
    displayName: 'Club',
    email: 'club@example.test',
    provider: 'imap',
    kind: 'email',
    color: '#654321',
    syncStatus: 'error',
    lastSyncError: 'connection reset',
  };
  return {
    gmail,
    accounts: [gmail, imap],
    emailAccounts: [gmail, imap],
    activeAccountId: 'gmail-account',
    load: vi.fn(),
    startReauth: vi.fn(),
    setNames: vi.fn(),
    isVisible: () => true,
  };
});
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => accounts }));
vi.mock('@/stores/connection', () => ({
  useConnectionStore: () => ({ applySyncSummary: vi.fn() }),
}));
vi.mock('@/stores/mailboxes', () => ({ useMailboxesStore: () => ({ load: vi.fn() }) }));
vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => ({ load: vi.fn(), threads: [] }),
}));
vi.mock('@/stores/ui', () => ({ useUiStore: () => ({ openAccountSetup: vi.fn(), settingsOpen: false, isSinglePane: true }) }));
vi.mock('@/stores/certificateTrust', () => ({
  useCertificateTrustStore: () => ({ problemFor: () => null, exceptionFor: () => null, openDetails: vi.fn() }),
}));
const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock('@/composables/useToast', () => ({ useToast: () => toast }));

vi.mock('./AccountVisibilitySettings.vue', () => ({ default: { template: '<div />' } }));
vi.mock('./AccountAppearanceSettings.vue', () => ({ default: { template: '<div />' } }));
vi.mock('./ProviderFeaturesEditor.vue', () => ({ __esModule: true, default: { template: '<div />' } }));
vi.mock('@/stores/accountQuota', () => ({ useAccountQuotaStore: () => ({ load: vi.fn(), quotaFor: () => null, levelFor: () => null }) }));
vi.mock('@/stores/workflowSync', () => ({ useWorkflowSyncStore: () => ({ load: vi.fn(), isLocalOnly: () => false, reasonFor: () => null }) }));
vi.mock('./AccountQuotaSettings.vue', () => ({ default: { template: '<div />' } }));

import AccountSetup from './AccountSetup.vue';

beforeEach(() => {
  vi.clearAllMocks();
  route.query = { section: 'accounts' };
});

describe('account list', () => {
  it('shows every account as one row with its state, and no options', () => {
    const wrapper = mount(AccountSetup);
    const rows = wrapper.findAll('[data-testid="account-row"]');

    expect(rows).toHaveLength(2);
    expect(rows[0]!.text()).toContain('Private');
    expect(rows[0]!.text()).toContain('person@example.test');
    expect(rows[1]!.text()).toContain('connection reset');
    expect(rows[1]!.find('.accounts__status--error').exists()).toBe(true);
    expect(wrapper.find('.password-form').exists()).toBe(false);
    expect(wrapper.text()).not.toContain('Renew sign-in');
  });

  it('opens an account on its own page, with Back to the list', async () => {
    const wrapper = mount(AccountSetup);
    await wrapper.findAll('[data-testid="account-row"]')[0]!.trigger('click');

    expect(router.push).toHaveBeenCalledWith({
      name: 'settings',
      query: { section: 'accounts', account: 'gmail-account' },
    });
  });
});

describe('account names', () => {
  it('saves the account name and the sender name apart', async () => {
    accounts.setNames.mockResolvedValue(true);
    route.query = { section: 'accounts', account: 'gmail-account' };
    const wrapper = mount(AccountSetup);

    expect((wrapper.get('[data-testid="account-label"]').element as HTMLInputElement).value).toBe('Private');
    expect((wrapper.get('[data-testid="account-sender-name"]').element as HTMLInputElement).value).toBe('Tom Example');
    await wrapper.get('[data-testid="account-label"]').setValue('Home');
    await wrapper.get('form.account-names').trigger('submit');
    await flushPromises();

    expect(accounts.setNames).toHaveBeenCalledWith('gmail-account', 'Home', 'Tom Example');
    expect(toast.success).toHaveBeenCalledWith('Names saved.');
  });

  it('offers Save only once something changed', async () => {
    route.query = { section: 'accounts', account: 'gmail-account' };
    const wrapper = mount(AccountSetup);
    expect(wrapper.find('.account-names__actions').exists()).toBe(false);

    await wrapper.get('[data-testid="account-sender-name"]').setValue('Tom');
    expect(wrapper.find('.account-names__actions').exists()).toBe(true);
  });
});

describe('Mail reconnect', () => {
  beforeEach(() => {
    route.query = { section: 'accounts', account: 'gmail-account' };
  });

  it('uses shared account recovery without requesting calendar access', async () => {
    accounts.startReauth.mockResolvedValue(undefined);
    const wrapper = mount(AccountSetup);
    const reconnect = wrapper.findAll('button').find((button) => button.text() === 'Renew sign-in');
    await reconnect!.trigger('click');
    await vi.waitFor(() => expect(accounts.startReauth).toHaveBeenCalledWith(accounts.gmail.id));
    expect(backend.backendSyncGoogleCalendar).not.toHaveBeenCalled();
    expect(wrapper.get('.status-msg').text()).toContain(accounts.gmail.email);
    // A success is a polite status line, not an alert.
    expect(wrapper.get('.status-msg').attributes('role')).toBe('status');
    expect(wrapper.get('.status-msg').classes()).not.toContain('status-msg--error');
  });

  it('shows a failed reconnect as an alert in the error tone', async () => {
    accounts.startReauth.mockRejectedValue(new Error('consent revoked'));
    const wrapper = mount(AccountSetup);
    await wrapper.findAll('button').find((button) => button.text() === 'Renew sign-in')!.trigger('click');

    await vi.waitFor(() =>
      expect(wrapper.get('.status-msg').classes()).toContain('status-msg--error'),
    );
    expect(wrapper.get('.status-msg').attributes('role')).toBe('alert');
  });
});
