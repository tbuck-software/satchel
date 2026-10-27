// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// #159: a rejected IMAP password had no way to be replaced at all.
const backend = vi.hoisted(() => ({ backendUpdateAccountPassword: vi.fn() }));
vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  backendCommandsAvailable: () => true,
  backendSupportsNativeCommands: () => true,
  ...backend,
}));

const route = vi.hoisted(() => ({ query: {} as Record<string, string> }));
vi.mock('vue-router', () => ({ useRoute: () => route }));

const accounts = vi.hoisted(() => {
  const account = {
    id: 'acc-work',
    displayName: 'Work',
    email: 'tom@example.test',
    provider: 'imap',
    kind: 'email',
    color: '#123456',
    syncStatus: 'error',
  };
  return { account, accounts: [account], emailAccounts: [account], activeAccountId: '', load: vi.fn() };
});
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => accounts }));
const connection = vi.hoisted(() => ({ applySyncSummary: vi.fn(), requestRetry: vi.fn() }));
vi.mock('@/stores/connection', () => ({ useConnectionStore: () => connection }));
vi.mock('@/stores/mailboxes', () => ({ useMailboxesStore: () => ({ load: vi.fn() }) }));
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => ({ load: vi.fn(), threads: [] }) }));
vi.mock('@/stores/ui', () => ({ useUiStore: () => ({ openAccountSetup: vi.fn() }) }));

vi.mock('./AccountVisibilitySettings.vue', () => ({ default: { template: '<div />' } }));

import AccountSetup from './AccountSetup.vue';

beforeEach(() => {
  vi.clearAllMocks();
  route.query = {};
  accounts.load.mockResolvedValue(undefined);
});

describe('replacing an account password', () => {
  it('opens the form for the account the banner named', async () => {
    route.query = { section: 'accounts', account: 'acc-work' };
    const wrapper = mount(AccountSetup, { attachTo: document.body });
    await flushPromises();

    expect(wrapper.get('.password-form').text()).toContain('New password for tom@example.test');
    expect(document.activeElement).toBe(wrapper.get('.password-form input').element);
    wrapper.unmount();
  });

  it('stores the checked password and syncs again', async () => {
    backend.backendUpdateAccountPassword.mockResolvedValue(undefined);
    const wrapper = mount(AccountSetup);
    await wrapper.get('button[title="Update password"]').trigger('click');

    await wrapper.get('.password-form input').setValue('correct horse');
    await wrapper.get('.password-form').trigger('submit');
    await flushPromises();

    expect(backend.backendUpdateAccountPassword).toHaveBeenCalledWith('acc-work', 'correct horse');
    expect(connection.requestRetry).toHaveBeenCalledTimes(1);
    expect(wrapper.find('.password-form').exists()).toBe(false);
    expect(wrapper.get('.status-msg').text()).toContain('Password for tom@example.test updated');
  });

  it('keeps the form and says why when the server refuses the new password', async () => {
    backend.backendUpdateAccountPassword.mockRejectedValue(new Error(JSON.stringify({
      key: 'backend.errors.passwordCheckFailed',
      params: { reason: 'imap login failed: No Response: [AUTHENTICATIONFAILED] Invalid credentials' },
    })));
    const wrapper = mount(AccountSetup);
    await wrapper.get('button[title="Update password"]').trigger('click');
    await wrapper.get('.password-form input').setValue('still wrong');
    await wrapper.get('.password-form').trigger('submit');
    await flushPromises();

    expect(wrapper.find('.password-form').exists()).toBe(true);
    expect(wrapper.get('.status-msg').text()).toContain('Signing in with the new password failed');
    expect(connection.requestRetry).not.toHaveBeenCalled();
  });
});
