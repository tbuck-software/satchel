// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  backendDiscoverAccount: vi.fn(),
  backendListOpenOauthFlows: vi.fn(),
  backendPollOauthFlow: vi.fn(),
  backendStartOauthFlow: vi.fn(),
  backendDeactivateAccount: vi.fn(),
  backendDetectMailProvider: vi.fn(async (): Promise<'gmail' | 'outlook' | null> => null),
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

beforeEach(() => {
  vi.clearAllMocks();
  // The poll loop waits between attempts; real timers would either hang the
  // test or, if made instant, spin into the next one.
  vi.useFakeTimers();
  accounts.load.mockResolvedValue(undefined);
  backend.backendListOpenOauthFlows.mockResolvedValue([]);
  backend.backendPollOauthFlow.mockResolvedValue({ status: 'pending' });
  backend.backendDeactivateAccount.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('onboarding as a modal surface', () => {
  it('is modal on first run too, and keeps Tab inside', async () => {
    const wrapper = mount(OnboardingFlow, { attachTo: document.body });
    await settle();

    const frame = wrapper.get('[role="dialog"]');
    expect(frame.attributes('aria-modal')).toBe('true');
    const focusables = frame.findAll('button:not([disabled])');
    expect(focusables.length).toBeGreaterThan(0);
    (focusables.at(-1)!.element as HTMLElement).focus();
    await frame.trigger('keydown', { key: 'Tab' });
    expect(frame.element.contains(document.activeElement)).toBe(true);
    wrapper.unmount();
  });
});

describe('onboarding after a process death during authorization', () => {
  /**
   * The verifier and the half-connected account both survived the restart.
   * Starting a second flow would strand the first one for good.
   */
  it('resumes the reported flow instead of starting a new one', async () => {
    backend.backendListOpenOauthFlows.mockResolvedValue([
      {
        state: 'flow-1',
        account_id: 'pending-account',
        provider: 'gmail',
        created_at: new Date().toISOString(),
        expires_at: new Date(Date.now() + 600_000).toISOString(),
      },
    ]);

    const wrapper = mount(OnboardingFlow);
    await settle();
    await vi.advanceTimersByTimeAsync(1_000);
    await wrapper.vm.$nextTick();

    expect(backend.backendStartOauthFlow).not.toHaveBeenCalled();
    expect(backend.backendPollOauthFlow).toHaveBeenCalledWith('flow-1');
    expect(wrapper.text()).toContain('Continuing the sign-in you started earlier.');
  });

  it('starts at step one when no flow is open', async () => {
    const wrapper = mount(OnboardingFlow);
    await settle();
    await vi.advanceTimersByTimeAsync(1_000);

    expect(backend.backendPollOauthFlow).not.toHaveBeenCalled();
    expect(wrapper.text()).not.toContain('Continuing the sign-in you started earlier.');
  });

  it('does not leave the half-connected account behind when the flow ended badly', async () => {
    backend.backendListOpenOauthFlows.mockResolvedValue([
      {
        state: 'flow-1',
        account_id: 'pending-account',
        provider: 'gmail',
        created_at: new Date().toISOString(),
        expires_at: new Date(Date.now() + 600_000).toISOString(),
      },
    ]);
    backend.backendPollOauthFlow.mockResolvedValue({
      status: 'failed',
      error: 'backend.errors.oauthAuthorizationCancelled',
    });

    mount(OnboardingFlow);
    await settle();
    await vi.advanceTimersByTimeAsync(1_000);
    await settle();

    expect(backend.backendDeactivateAccount).toHaveBeenCalledWith('pending-account');
  });
});

describe('onboarding when the server turns the credentials down', () => {
  /**
   * The wizard used to print `(error as Error).message`, so a first-time user
   * met a Rust chain at the one moment they have the least idea what to do.
   */
  async function failDiscoveryWith(chain: string) {
    backend.backendDiscoverAccount.mockRejectedValue(new Error(chain));
    const wrapper = mount(OnboardingFlow);
    await settle();
    await wrapper.find('input[type="email"]').setValue('person@example.test');
    await wrapper.find('input[type="password"]').setValue('hunter2');
    await wrapper.find('form').trigger('submit');
    await settle();
    await wrapper.vm.$nextTick();
    return wrapper;
  }

  it('shows the translated cause and keeps the chain in a collapsed details', async () => {
    const chain = 'imap: login failed: [AUTHENTICATIONFAILED] Invalid credentials (Failure)';
    const wrapper = await failDiscoveryWith(chain);

    expect(wrapper.find('.inline-error').text())
      .toContain('The server rejected this user name and password');
    // #165: announced, not just painted red.
    expect(wrapper.find('.inline-error').attributes('role')).toBe('alert');
    const details = wrapper.find('details.error-detail');
    expect(details.exists()).toBe(true);
    expect(details.attributes('open')).toBeUndefined();
    expect(details.find('pre').text()).toBe(chain);
    expect(wrapper.find('.inline-error').text()).not.toContain('AUTHENTICATIONFAILED');
  });

  it('tells a certificate problem apart from a rejected password', async () => {
    const wrapper = await failDiscoveryWith(
      'imap: connect: tls handshake: invalid peer certificate: UnknownIssuer',
    );

    expect(wrapper.find('.inline-error').text())
      .toContain('The encrypted connection could not be established');
  });
});
