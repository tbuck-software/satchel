// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, expect, it, vi } from 'vitest';
import ServerRulesSettings from './ServerRulesSettings.vue';

const backend = vi.hoisted(() => ({
  backendGetServerRulesOverview: vi.fn(),
  backendDetectServerRules: vi.fn(),
  backendEnableServerRules: vi.fn(),
  backendDisableServerRules: vi.fn(),
  backendSetServerRulesEndpoint: vi.fn(),
  translateBackendError: vi.fn((_error: unknown): string | null => null),
}));
const confirm = vi.hoisted(() => ({ ask: vi.fn() }));
vi.mock('@/services/backend', () => backend);
vi.mock('@/composables/useConfirm', () => ({ useConfirm: () => confirm }));

function account(overrides: Record<string, unknown> = {}) {
  return {
    account_id: 'acc',
    display_name: 'Work',
    email: 'me@example.com',
    availability: 'available',
    host: 'imap.example.com',
    port: 4190,
    tls: 'starttls',
    source: 'imap_host',
    host_override: null,
    port_override: null,
    tls_override: null,
    implementation: 'Dovecot',
    extensions: ['include', 'imap4flags', 'vacation'],
    checked_at: null,
    enabled: false,
    server_active: false,
    activation_mode: null,
    previous_active_script: null,
    consent_script: null,
    include_available: true,
    vacation_available: true,
    vacation_window_available: false,
    pushed_vacation: false,
    pushed_rule_count: 0,
    last_push_at: null,
    last_error: null,
    ...overrides,
  };
}

function button(wrapper: ReturnType<typeof mount>, text: string) {
  const found = wrapper.findAll('button').find((candidate) => candidate.text() === text);
  if (!found) throw new Error(`no button ${text}: ${wrapper.findAll('button').map((b) => b.text()).join(', ')}`);
  return found;
}

beforeEach(() => {
  vi.resetAllMocks();
  backend.translateBackendError.mockReturnValue(null);
});

it('turns server rules on without taking over another script', async () => {
  backend.backendGetServerRulesOverview.mockResolvedValue({ accounts: [account()], rules: [] });
  backend.backendEnableServerRules.mockResolvedValue({
    accounts: [account({ enabled: true, consent_script: 'roundcube' })],
    rules: [],
  });
  const wrapper = mount(ServerRulesSettings);
  await flushPromises();
  expect(wrapper.text()).toContain('Available');
  expect(wrapper.text()).toContain('imap.example.com:4190 · STARTTLS');

  await button(wrapper, 'Turn on').trigger('click');
  await flushPromises();
  expect(backend.backendEnableServerRules).toHaveBeenCalledWith('acc', 'ask');
  expect(wrapper.text()).toContain('Your script “roundcube” is active on the server');
  expect(wrapper.text()).toContain('Needs your choice');
});

it('runs both scripts only after an explicit confirmation', async () => {
  backend.backendGetServerRulesOverview.mockResolvedValue({
    accounts: [account({ enabled: true, consent_script: 'roundcube' })],
    rules: [],
  });
  backend.backendEnableServerRules.mockResolvedValue({
    accounts: [account({ enabled: true, server_active: true, activation_mode: 'wrapper', previous_active_script: 'roundcube', pushed_rule_count: 2 })],
    rules: [],
  });
  const wrapper = mount(ServerRulesSettings);
  await flushPromises();

  confirm.ask.mockResolvedValueOnce(false);
  await button(wrapper, 'Run both').trigger('click');
  await flushPromises();
  expect(backend.backendEnableServerRules).not.toHaveBeenCalled();

  confirm.ask.mockResolvedValueOnce(true);
  await button(wrapper, 'Run both').trigger('click');
  await flushPromises();
  expect(backend.backendEnableServerRules).toHaveBeenCalledWith('acc', 'wrapper');
  expect(wrapper.text()).toContain('Active on server');
  expect(wrapper.text()).toContain('2 rules run on the server');
  expect(wrapper.text()).toContain('alongside your script “roundcube”');
});

it('offers no wrapper when the server lacks include, and keeps rules local on request', async () => {
  backend.backendGetServerRulesOverview.mockResolvedValue({
    accounts: [account({ enabled: true, consent_script: 'main', include_available: false })],
    rules: [],
  });
  backend.backendDisableServerRules.mockResolvedValue({ accounts: [account()], rules: [] });
  const wrapper = mount(ServerRulesSettings);
  await flushPromises();
  expect(wrapper.findAll('button').some((b) => b.text() === 'Run both')).toBe(false);
  await button(wrapper, 'Keep rules in Satchel').trigger('click');
  await flushPromises();
  expect(backend.backendDisableServerRules).toHaveBeenCalledWith('acc');
});

it('saves a manual server address', async () => {
  backend.backendGetServerRulesOverview.mockResolvedValue({ accounts: [account({ availability: 'unavailable' })], rules: [] });
  backend.backendSetServerRulesEndpoint.mockResolvedValue({
    accounts: [account({ availability: 'unknown', host: 'sieve.example.com', host_override: 'sieve.example.com', port: 2000, tls: 'tls' })],
    rules: [],
  });
  const wrapper = mount(ServerRulesSettings);
  await flushPromises();
  expect(wrapper.text()).toContain('Not available');
  await button(wrapper, 'Enter server manually').trigger('click');
  const inputs = wrapper.findAll('form input');
  await inputs[0]!.setValue(' sieve.example.com ');
  await inputs[1]!.setValue('2000');
  await button(wrapper, 'TLS').trigger('click');
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  expect(backend.backendSetServerRulesEndpoint).toHaveBeenCalledWith({
    accountId: 'acc', host: 'sieve.example.com', port: 2000, tls: 'tls',
  });
  expect(wrapper.text()).toContain('sieve.example.com:2000 · TLS');
});

it('shows a stored push error as text', async () => {
  backend.translateBackendError.mockImplementation((error: unknown) =>
    (error as { key?: string }).key === 'backend.errors.serverRules.authFailed' ? 'Password refused.' : null);
  backend.backendGetServerRulesOverview.mockResolvedValue({
    accounts: [account({ enabled: true, last_error: 'backend.errors.serverRules.authFailed' })],
    rules: [],
  });
  const wrapper = mount(ServerRulesSettings);
  await flushPromises();
  expect(wrapper.text()).toContain('Password refused.');
});
