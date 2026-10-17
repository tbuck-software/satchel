// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  backendStartOauthFlow: vi.fn(),
  backendPollOauthFlow: vi.fn(),
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
  backend.backendStartOauthFlow.mockResolvedValue({ state: 'flow', browser_opened: true });
  backend.backendPollOauthFlow.mockResolvedValue({ status: 'completed' });
  accounts.load.mockResolvedValue(undefined);
  vi.spyOn(window, 'setTimeout').mockImplementation((callback: TimerHandler) => {
    if (typeof callback === 'function') callback();
    return 1 as never;
  });
});

describe('Google Calendar reconnect errors', () => {
  it('translates an encoded backend error instead of displaying its JSON', async () => {
    const encoded = JSON.stringify({
      key: 'backend.errors.googleCalendarSyncFailed',
      params: { account: accounts.account.email },
    });
    backend.backendSyncGoogleCalendar.mockResolvedValue({
      accounts_checked: 1,
      events_synced: 0,
      errors: [encoded],
    });
    const wrapper = mount(AccountSetup);

    const reconnect = wrapper.findAll('button.icon-button')[0];
    expect(reconnect).toBeDefined();
    await reconnect!.trigger('click');

    await vi.waitFor(() =>
      expect(wrapper.get('.status-msg').text()).toContain(
        `Google Calendar for ${accounts.account.email} could not be synchronized.`,
      ),
    );
    expect(wrapper.get('.status-msg').text()).not.toContain('{"key"');
  });
});
