// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';

const backend = vi.hoisted(() => ({
  backendDiagnosticsOverview: vi.fn(),
  backendAppBuildInfo: vi.fn(),
  backendDiagnosticsReport: vi.fn(),
  backendOpenLogsFolder: vi.fn(),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  translateBackendError: vi.fn((error: unknown) => {
    if (typeof error !== 'string') return null;
    try {
      const payload = JSON.parse(error) as { key?: string };
      if (payload.key === 'backend.errors.accountSyncFailed') return 'Translated sync failure';
      if (payload.key === 'backend.errors.queueActionFailed') return 'Translated queue failure';
      return null;
    } catch {
      return null;
    }
  }),
}));
vi.mock('@/services/backend', () => backend);

import DiagnosticsPane from './DiagnosticsPane.vue';
import { useToast } from '@/composables/useToast';

const NOW = Date.parse('2026-08-27T12:00:00Z');

const EMPTY = {
  generated_at: '2026-08-27T12:00:00Z',
  accounts: [],
  queue: { by_status: [], recent_failures: [] },
  notifications: { by_status: [], recent: [] },
  notification_outbox: { by_state: [], reasons_24h: [], recent: [] },
  sync_leases: [],
  rate_limits: [],
};

const POPULATED = {
  generated_at: '2026-08-27T12:00:00Z',
  accounts: [
    {
      id: 'acc-ok',
      email: 'ruhig@example.de',
      display_name: 'Ruhig',
      provider: 'imap',
      sync_status: 'idle',
      last_sync_at: '2026-08-27T11:58:00Z',
      last_sync_error: null,
      last_sync_error_at: null,
      last_oauth_error: null,
      last_oauth_error_at: null,
    },
    {
      id: 'acc-bad',
      email: 'kaputt@example.de',
      display_name: 'Kaputt',
      provider: 'gmail',
      sync_status: 'error',
      last_sync_at: '2026-08-27T09:00:00Z',
      last_sync_error: 'IMAP LOGIN failed: AUTHENTICATIONFAILED',
      last_sync_error_at: '2026-08-27T11:30:00Z',
      last_oauth_error: null,
      last_oauth_error_at: null,
    },
  ],
  queue: {
    by_status: [
      { status: 'failed', entries: 2, total_retries: 9, max_retries: 5 },
      { status: 'pending', entries: 3, total_retries: 1, max_retries: 1 },
    ],
    recent_failures: [
      {
        id: 'q-1',
        account_id: 'acc-bad',
        action_type: 'send_message',
        status: 'failed',
        error: 'SMTP 550 rejected',
        attempts: 5,
        next_due: null,
        updated_at: '2026-08-27T11:45:00Z',
      },
    ],
  },
  notifications: {
    by_status: [{ status: 'pending', entries: 1, total_retries: 0, max_retries: 0 }],
    recent: [
      {
        id: 'notification:abc',
        account_id: 'acc-ok',
        action_type: 'remote_mutation',
        status: 'pending',
        error: null,
        attempts: 0,
        next_due: null,
        updated_at: '2026-08-27T11:59:30Z',
      },
    ],
  },
  notification_outbox: {
    by_state: [
      { state: 'delivered', entries: 4 },
      { state: 'dismissed', entries: 3 },
    ],
    reasons_24h: [
      { reason: 'filter_already_read', entries: 2 },
      { reason: 'filter_not_inbox', entries: 1 },
      { reason: 'some_new_code', entries: 1 },
    ],
    recent: [
      {
        message_id: 'm-1',
        state: 'dismissed',
        created_at: '2026-08-27T11:40:00Z',
        delivered_at: '2026-08-27T11:41:00Z',
        reason: 'filter_already_read',
        detail: 'chef@example.de',
        subject: 'Quartalszahlen',
        from_name: 'Die Chefin',
        from_address: 'chef@example.de',
        date: '2026-08-27T11:39:00Z',
        account_id: 'acc-ok',
      },
      {
        message_id: 'm-2',
        state: 'delivered',
        created_at: '2026-08-27T11:50:00Z',
        delivered_at: '2026-08-27T11:50:05Z',
        reason: null,
        detail: null,
        subject: null,
        from_name: null,
        from_address: 'noreply@example.de',
        date: '2026-08-27T11:49:00Z',
        account_id: 'acc-ok',
      },
    ],
  },
  sync_leases: [
    {
      account_id: 'acc-lease',
      email: 'lease@example.de',
      owner_token: 'worker-42',
      lease_expires_at: '2026-08-27T11:55:00Z',
      updated_at: '2026-08-27T11:50:00Z',
      expired: true,
    },
  ],
  rate_limits: [
    {
      provider: 'gmail',
      blocked_until: '2026-08-27T12:30:00Z',
      streak: 3,
      updated_at: '2026-08-27T11:55:00Z',
    },
  ],
};

afterEach(() => {
  vi.useRealTimers();
});

beforeEach(() => {
  vi.clearAllMocks();
  // Only Date is pinned, not the timer queue — `vi.waitFor` still needs to tick.
  vi.setSystemTime(NOW);
  backend.backendSupportsNativeCommands.mockReturnValue(true);
  backend.backendDiagnosticsOverview.mockResolvedValue(structuredClone(EMPTY));
  backend.backendAppBuildInfo.mockResolvedValue({
    version: '0.1.0', commit: 'abc123def456', os: 'macos', arch: 'aarch64', debug: false,
  });
  backend.backendDiagnosticsReport.mockResolvedValue('Satchel diagnostics (redacted)');
  backend.backendOpenLogsFolder.mockResolvedValue(undefined);
  useToast().clear();
});

async function mountPane() {
  const wrapper = mount(DiagnosticsPane);
  await vi.waitFor(() => expect(backend.backendDiagnosticsOverview).toHaveBeenCalled());
  await wrapper.vm.$nextTick();
  return wrapper;
}

describe('an installation with nothing wrong', () => {
  it('says so in every section instead of showing empty tables', async () => {
    const wrapper = await mountPane();

    expect(wrapper.text()).toContain('No sync errors');
    expect(wrapper.text()).toContain('Queue is empty');
    expect(wrapper.text()).toContain('No pending alert actions');
    expect(wrapper.text()).toContain('No notification entries');
    expect(wrapper.text()).toContain('No sync leases');
    expect(wrapper.text()).toContain('No blocks active');
    expect(wrapper.find('table').exists()).toBe(false);
  });
});

describe('an installation with problems', () => {
  beforeEach(() => {
    backend.backendDiagnosticsOverview.mockResolvedValue(structuredClone(POPULATED));
  });

  /** A healthy account in the list would bury the one that needs attention. */
  it('lists only the accounts that actually failed', async () => {
    const wrapper = await mountPane();

    expect(wrapper.text()).toContain('kaputt@example.de');
    expect(wrapper.text()).not.toContain('ruhig@example.de');
    expect(wrapper.text()).toContain('IMAP LOGIN failed: AUTHENTICATIONFAILED');
  });

  it('translates structured sync errors while preserving legacy diagnostics', async () => {
    const populated = structuredClone(POPULATED);
    populated.accounts[1]!.last_sync_error = JSON.stringify({
      key: 'backend.errors.accountSyncFailed',
      params: { account: 'kaputt@example.de', message: 'provider detail' },
    });
    backend.backendDiagnosticsOverview.mockResolvedValue(populated);

    const wrapper = await mountPane();

    expect(wrapper.text()).toContain('Translated sync failure');
    expect(wrapper.text()).not.toContain('provider detail');
  });

  it('shows queue counts and retry totals per status', async () => {
    const wrapper = await mountPane();
    const table = wrapper.find('table');

    expect(table.text()).toContain('Failed');
    expect(table.text()).toContain('Pending');
    expect(wrapper.text()).toContain('SMTP 550 rejected');
  });

  it('translates structured queue failures', async () => {
    const populated = structuredClone(POPULATED);
    populated.queue.recent_failures[0]!.error = JSON.stringify({
      key: 'backend.errors.queueActionFailed',
      params: { message: 'provider detail' },
    });
    backend.backendDiagnosticsOverview.mockResolvedValue(populated);

    const wrapper = await mountPane();

    expect(wrapper.text()).toContain('Translated queue failure');
    expect(wrapper.text()).not.toContain('provider detail');
  });

  it('reports notification actions separately from the rest of the queue', async () => {
    const wrapper = await mountPane();
    const actions = wrapper.get('[data-testid="notification-actions"]');

    expect(actions.text()).toContain('Alert actions');
    expect(actions.text()).toContain('remote_mutation');
  });

  it('explains why each recent message did or did not notify', async () => {
    const wrapper = await mountPane();
    const outbox = wrapper.get('[data-testid="notification-outbox"]');

    expect(outbox.text()).toContain('Delivered');
    expect(outbox.text()).toContain('Dismissed');
    expect(outbox.text()).toContain('Quartalszahlen');
    expect(outbox.text()).toContain('Die Chefin');
    expect(outbox.text()).toContain('Already read');
    expect(outbox.text()).toContain('(no subject)');
    expect(outbox.text()).toContain('noreply@example.de');
  });

  /** The histogram labels known codes and shows unknown ones verbatim. */
  it('histograms the reason codes of the last day', async () => {
    const wrapper = await mountPane();
    const rows = wrapper.findAll('.histogram__row');
    const texts = rows.map((row) => row.text());

    expect(texts).toHaveLength(3);
    expect(texts[0]).toContain('Already read');
    expect(texts[0]).toContain('2');
    expect(texts[1]).toContain('Not in inbox');
    expect(texts[2]).toContain('some_new_code');
    const widths = rows.map((row) => row.get('.histogram__bar').attributes('style'));
    expect(widths[0]).toContain('50%');
  });

  it('lists sync leases with owner and expiry', async () => {
    const wrapper = await mountPane();
    const leases = wrapper.get('[data-testid="sync-leases"]');

    expect(leases.text()).toContain('lease@example.de');
    expect(leases.text()).toContain('worker-42');
    expect(leases.text()).toContain('Expired');
  });

  it('shows an armed rate-limit block with its streak', async () => {
    const wrapper = await mountPane();

    expect(wrapper.text()).toContain('gmail');
    expect(wrapper.text()).toContain('Blocked');
    expect(wrapper.text()).toContain('Blocks in a row');
  });

  /** Relative and forward-looking for a block that has not lapsed. */
  it('renders timestamps relative to now', async () => {
    const wrapper = await mountPane();

    expect(wrapper.text()).toContain('30 min ago');
    expect(wrapper.text()).toContain('in 30 min');
  });
});

describe('the refresh button', () => {
  it('asks the backend again', async () => {
    const wrapper = await mountPane();

    await wrapper.get('button').trigger('click');
    await vi.waitFor(() =>
      expect(backend.backendDiagnosticsOverview).toHaveBeenCalledTimes(2),
    );
  });
});

describe('a backend that refuses', () => {
  it('shows the error instead of an empty pane', async () => {
    backend.backendDiagnosticsOverview.mockRejectedValue(new Error('database is locked'));
    const wrapper = await mountPane();

    expect(wrapper.text()).toContain('database is locked');
  });
});

describe('build and bug report', () => {
  function buttonNamed(wrapper: Awaited<ReturnType<typeof mountPane>>, label: string) {
    const button = wrapper.findAll('button').find((candidate) => candidate.text() === label);
    if (!button) throw new Error(`no button "${label}"`);
    return button;
  }

  it('names the running version and commit', async () => {
    const wrapper = await mountPane();
    await vi.waitFor(() => expect(wrapper.find('[data-testid="build-info"]').exists()).toBe(true));

    const build = wrapper.get('[data-testid="build-info"]').text();
    expect(build).toContain('0.1.0');
    expect(build).toContain('abc123def456');
    expect(build).toContain('macos/aarch64');
  });

  it('copies the redacted report the backend built', async () => {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const wrapper = await mountPane();

    await buttonNamed(wrapper, 'Copy diagnostics (redacted)').trigger('click');

    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith('Satchel diagnostics (redacted)'));
    expect(useToast().state.toasts.at(-1)?.tone).toBe('success');
  });

  it('opens the logs folder on the desktop only', async () => {
    const wrapper = await mountPane();
    await vi.waitFor(() => expect(wrapper.text()).toContain('Open logs folder'));

    await buttonNamed(wrapper, 'Open logs folder').trigger('click');
    expect(backend.backendOpenLogsFolder).toHaveBeenCalledOnce();

    backend.backendAppBuildInfo.mockResolvedValue({
      version: '0.1.0', commit: 'abc123def456', os: 'android', arch: 'aarch64', debug: false,
    });
    const phone = await mountPane();
    await vi.waitFor(() => expect(phone.find('[data-testid="build-info"]').exists()).toBe(true));
    expect(phone.text()).not.toContain('Open logs folder');
  });
});
