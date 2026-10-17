// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  backendAddAccount: vi.fn(),
  backendCompleteOauthFlow: vi.fn(),
  backendDeactivateAccount: vi.fn(),
  backendDetectMailProvider: vi.fn(async (): Promise<'gmail' | 'outlook' | null> => null),
  backendDiscoverAccount: vi.fn(),
  backendListOpenOauthFlows: vi.fn(),
  backendPollOauthFlow: vi.fn(),
  backendStartOauthFlow: vi.fn(),
  backendSyncGroupware: vi.fn(),
  backendTestImapConnection: vi.fn(),
  backendUpsertSignature: vi.fn(),
}));
vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  backendCommandsAvailable: () => true,
  backendSupportsNativeCommands: () => true,
  ...backend,
}));

const accounts = vi.hoisted(() => ({
  accounts: [
    { id: 'pending-account', displayName: 'Gmail', email: 'person@example.test', provider: 'gmail' },
  ],
  activeAccountId: '',
  load: vi.fn(),
  setActiveAccount: vi.fn(),
}));
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => accounts }));
vi.mock('@/stores/contacts', () => ({
  useContactsStore: () => ({ syncAvatars: vi.fn(), lastAvatarSyncError: '' }),
}));
vi.mock('@/stores/settings', () => ({
  useSettingsStore: () => ({
    onboardingComplete: false,
    theme: 'dark',
    vimModeEnabled: false,
    setTheme: vi.fn(),
    setVimMode: vi.fn(),
    completeOnboarding: vi.fn(),
  }),
}));
vi.mock('@/stores/ui', () => ({
  useUiStore: () => ({ accountSetupOpen: false, closeAccountSetup: vi.fn() }),
}));

import OnboardingFlow from './OnboardingFlow.vue';

/** Let the mounted component's own promise chain settle. */
async function settle() {
  for (let tick = 0; tick < 12; tick += 1) await Promise.resolve();
}

const OPEN_FLOW = {
  state: 'flow-1',
  account_id: 'pending-account',
  provider: 'gmail',
  created_at: new Date().toISOString(),
  expires_at: new Date(Date.now() + 600_000).toISOString(),
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers();
  accounts.load.mockResolvedValue(undefined);
  backend.backendListOpenOauthFlows.mockResolvedValue([]);
  backend.backendPollOauthFlow.mockResolvedValue({ status: 'pending' });
  backend.backendDeactivateAccount.mockResolvedValue(undefined);
  backend.backendUpsertSignature.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

/** Run a resumed flow into its two-minute timeout. */
async function timeOutAResumedFlow() {
  backend.backendListOpenOauthFlows.mockResolvedValue([OPEN_FLOW]);
  const wrapper = mount(OnboardingFlow);
  await settle();
  await vi.advanceTimersByTimeAsync(130_000);
  await settle();
  await wrapper.vm.$nextTick();
  return wrapper;
}

describe('finishing an OAuth sign-in by hand', () => {
  /**
   * The loopback callback is the only automatic way back in. When a firewall
   * or a remote desktop eats it, the wizard used to time out, throw the
   * account away and leave the user with nothing to try.
   */
  it('offers the state and code fields instead of discarding the account', async () => {
    const wrapper = await timeOutAResumedFlow();

    expect(wrapper.text()).toContain('Finish the sign-in by hand');
    expect(backend.backendDeactivateAccount).not.toHaveBeenCalled();
    const state = wrapper.findAll('input').find(
      (input) => (input.element as HTMLInputElement).value === 'flow-1',
    );
    expect(state, 'the state of the open flow is prefilled').toBeDefined();
  });

  it('spends the pasted pair and finishes the account', async () => {
    backend.backendCompleteOauthFlow.mockResolvedValue({});
    const wrapper = await timeOutAResumedFlow();

    const inputs = wrapper.findAll('.manual-card input');
    await inputs[1]!.setValue('4/0Ax-code');
    const complete = wrapper.findAll('button').find(
      (button) => button.text() === 'Complete sign-in',
    );
    expect(complete).toBeDefined();
    await complete!.trigger('click');
    await settle();

    expect(backend.backendCompleteOauthFlow).toHaveBeenCalledWith({
      state: 'flow-1',
      code: '4/0Ax-code',
    });
    expect(backend.backendDeactivateAccount).not.toHaveBeenCalled();
  });

  it('does not call the backend before both fields are filled', async () => {
    const wrapper = await timeOutAResumedFlow();

    const complete = wrapper.findAll('button').find(
      (button) => button.text() === 'Complete sign-in',
    );
    expect(complete!.attributes('disabled')).toBeDefined();
    expect(backend.backendCompleteOauthFlow).not.toHaveBeenCalled();
  });

  /**
   * The backend consumes the PKCE session on every attempt. A wrong code is
   * therefore terminal — say so and clean up rather than offering a retry
   * that cannot work.
   */
  it('reports a refused pair and clears the half-connected account', async () => {
    backend.backendCompleteOauthFlow.mockRejectedValue(new Error('invalid_grant'));
    const wrapper = await timeOutAResumedFlow();

    const inputs = wrapper.findAll('.manual-card input');
    await inputs[1]!.setValue('wrong');
    await wrapper.findAll('button')
      .find((button) => button.text() === 'Complete sign-in')!
      .trigger('click');
    await settle();
    await wrapper.vm.$nextTick();

    expect(backend.backendDeactivateAccount).toHaveBeenCalledWith('pending-account');
    expect(wrapper.find('.inline-error').text())
      .toContain('The stored sign-in is no longer accepted');
  });
});

describe('testing a hand-entered IMAP server', () => {
  async function openTheManualCard() {
    backend.backendDiscoverAccount.mockRejectedValue(new Error('connection refused'));
    const wrapper = mount(OnboardingFlow);
    await settle();
    await wrapper.find('input[type="email"]').setValue('person@example.test');
    await wrapper.find('input[type="password"]').setValue('hunter2');
    await wrapper.find('form').trigger('submit');
    await settle();
    await wrapper.vm.$nextTick();
    return wrapper;
  }

  function testButton(wrapper: ReturnType<typeof mount>) {
    return wrapper.findAll('button').find((button) => button.text() === 'Test connection');
  }

  it('sends the typed host and port and names the outcome', async () => {
    backend.backendTestImapConnection.mockResolvedValue({
      ok: true,
      folders: 7,
      folder_sample: [],
    });
    const wrapper = await openTheManualCard();

    const inputs = wrapper.findAll('.manual-card input');
    await inputs[1]!.setValue('imap.example.test');
    await inputs[2]!.setValue('993');
    await testButton(wrapper)!.trigger('click');
    await settle();
    await wrapper.vm.$nextTick();

    expect(backend.backendTestImapConnection).toHaveBeenCalledWith({
      email: 'person@example.test',
      config: {
        host: 'imap.example.test',
        port: 993,
        tls: true,
        username: 'person@example.test',
        password: 'hunter2',
      },
    });
    expect(wrapper.find('.inline-success').text()).toContain('7 folders found');
  });

  it('names the class of failure rather than the Rust chain', async () => {
    backend.backendTestImapConnection.mockRejectedValue(
      new Error('imap: connect: tls handshake: invalid peer certificate: UnknownIssuer'),
    );
    const wrapper = await openTheManualCard();

    const inputs = wrapper.findAll('.manual-card input');
    await inputs[1]!.setValue('imap.example.test');
    await testButton(wrapper)!.trigger('click');
    await settle();
    await wrapper.vm.$nextTick();

    expect(wrapper.find('.inline-error').text())
      .toContain('The encrypted connection could not be established');
    expect(wrapper.find('.inline-error').text()).not.toContain('UnknownIssuer');
    expect(wrapper.find('.inline-success').exists()).toBe(false);
  });
});
