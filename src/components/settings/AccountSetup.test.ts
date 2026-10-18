// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  backendSyncGoogleCalendar: vi.fn(),
}));
vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  backendCommandsAvailable: () => true,
  backendSupportsNativeCommands: () => true,
  ...backend,
}));

const accounts = vi.hoisted(() => {
  const account = {
    id: 'gmail-account',
    displayName: 'Gmail',
    email: 'person@example.test',
    provider: 'gmail',
    kind: 'email',
    color: '#123456',
    syncStatus: 'idle',
  };
  return {
    account,
    accounts: [account],
    emailAccounts: [account],
    activeAccountId: 'gmail-account',
    load: vi.fn(),
    startReauth: vi.fn(),
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
vi.mock('@/stores/ui', () => ({ useUiStore: () => ({ openAccountSetup: vi.fn() }) }));

import AccountSetup from './AccountSetup.vue';

beforeEach(() => {
  vi.clearAllMocks();
});

describe('Mail reconnect', () => {
  it('uses shared account recovery without requesting calendar access', async () => {
    accounts.startReauth.mockResolvedValue(undefined);
    const wrapper = mount(AccountSetup);
    const reconnect = wrapper.findAll('button.icon-button')[0];
    await reconnect!.trigger('click');
    await vi.waitFor(() => expect(accounts.startReauth).toHaveBeenCalledWith(accounts.account.id));
    expect(backend.backendSyncGoogleCalendar).not.toHaveBeenCalled();
    expect(wrapper.get('.status-msg').text()).toContain(accounts.account.email);
  });
});
