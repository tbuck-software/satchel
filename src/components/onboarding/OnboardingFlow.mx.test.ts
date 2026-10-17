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

// #165: name@company.com on Google Workspace got the IMAP password form, which Google refuses.
beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers();
  accounts.load.mockResolvedValue(undefined);
  backend.backendListOpenOauthFlows.mockResolvedValue([]);
  backend.backendAddAccount.mockResolvedValue({ id: 'new-account' });
  backend.backendStartOauthFlow.mockReturnValue(new Promise(() => {}));
});

afterEach(() => {
  vi.useRealTimers();
});

async function typeEmail(wrapper: ReturnType<typeof mount>, email: string) {
  await wrapper.find('input[type="email"]').setValue(email);
  await vi.advanceTimersByTimeAsync(400);
  await settle();
}

describe('company domains behind Google Workspace or Microsoft 365', () => {
  it('preselects Google sign-in when the MX records point at Google', async () => {
    backend.backendDetectMailProvider.mockResolvedValue('gmail');
    const wrapper = mount(OnboardingFlow);
    await settle();

    await typeEmail(wrapper, 'anna@acme.example');

    expect(backend.backendDetectMailProvider).toHaveBeenCalledWith('acme.example');
    expect(wrapper.text()).toContain('acme.example receives mail through Google Workspace');
    expect(wrapper.find('input[type="password"]').exists()).toBe(false);
  });

  it('names Microsoft 365 for a domain on Exchange Online', async () => {
    backend.backendDetectMailProvider.mockResolvedValue('outlook');
    const wrapper = mount(OnboardingFlow);
    await settle();

    await typeEmail(wrapper, 'anna@contoso.example');

    expect(wrapper.text()).toContain('contoso.example receives mail through Microsoft 365');
  });

  it('keeps the password form for a domain that hosts its own mail', async () => {
    backend.backendDetectMailProvider.mockResolvedValue(null);
    const wrapper = mount(OnboardingFlow);
    await settle();

    await typeEmail(wrapper, 'anna@self-hosted.example');

    expect(wrapper.find('input[type="password"]').exists()).toBe(true);
    expect(wrapper.text()).toContain('self-hosted.example');
  });

  it('does not ask the resolver about a consumer domain it already knows', async () => {
    const wrapper = mount(OnboardingFlow);
    await settle();

    await typeEmail(wrapper, 'anna@gmail.com');

    expect(backend.backendDetectMailProvider).not.toHaveBeenCalled();
  });

  it('waits for a pending lookup when submitted early, and signs in with OAuth', async () => {
    let answer!: (value: 'gmail') => void;
    backend.backendDetectMailProvider.mockReturnValue(new Promise((resolve) => { answer = resolve; }));
    const wrapper = mount(OnboardingFlow);
    await settle();
    await wrapper.find('input[type="email"]').setValue('anna@acme.example');
    await wrapper.find('input[type="password"]').setValue('typed-before-the-answer');

    await wrapper.find('form').trigger('submit');
    await settle();
    expect(backend.backendDiscoverAccount).not.toHaveBeenCalled();

    answer('gmail');
    await settle();

    expect(backend.backendDiscoverAccount).not.toHaveBeenCalled();
    expect(backend.backendAddAccount).toHaveBeenCalledWith(expect.objectContaining({ provider: 'gmail' }));
  });
});
