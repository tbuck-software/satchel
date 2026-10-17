// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendPgpKey, BackendPlugin, BackendWebhook } from '@/services/backend';

// The key list only renders once the PGP plugin is installed.
const pgpPlugin: BackendPlugin = {
  id: 'satchel-plugin-pgp',
  version: '1.0.0',
  enabled: true,
  api_version: '1',
  capabilities: [],
  event_subscriptions: [],
  ui_slots: [],
  trust: 'verified' as const,
  signature_verified: true,
  manifest_errors: [],
  granted_capabilities: [],
  enforced_capabilities: ['read_threads'],
};

const backend = vi.hoisted(() => ({
  backendErrorKey: vi.fn(() => null),
  backendDeletePgpKey: vi.fn(),
  backendGeneratePgpKey: vi.fn(),
  backendImportPgpKeys: vi.fn(),
  backendListPluginAuditLog: vi.fn(async () => []),
  backendListPluginMetrics: vi.fn(async () => []),
  backendListPgpKeys: vi.fn(),
  backendPgpEngineStatus: vi.fn(async () => ({ engine: 'builtin', available: true })),
  backendListPlugins: vi.fn(async () => [pgpPlugin]),
  backendDeleteWebhook: vi.fn(),
  backendListWebhooks: vi.fn(async (): Promise<unknown[]> => []),
  backendSetPluginEnabled: vi.fn(async () => {}),
  backendSetPluginPermission: vi.fn(),
  backendUpsertWebhook: vi.fn(),
  backendUnlockPgpKeys: vi.fn(),
  backendValidatePgpBundlePassphrase: vi.fn(),
  translateBackendError: vi.fn(() => null),
}));
const toast = vi.hoisted(() => ({
  info: vi.fn(),
  success: vi.fn(),
  warning: vi.fn(),
  error: vi.fn(),
}));

vi.mock('@/services/backend', () => backend);
vi.mock('@/composables/useToast', () => ({ useToast: () => toast }));

import PluginManager from './PluginManager.vue';

function pgpKey(overrides: Partial<BackendPgpKey> = {}): BackendPgpKey {
  return {
    id: 'key-1',
    label: 'Alex (privat)',
    key_type: 'private',
    fingerprint: 'AAAABBBBCCCCDDDDEEEEFFFF0000111122223333',
    user_ids: ['alex@example.com'],
    has_passphrase: true,
    is_unlocked: false,
    is_expired: false,
    can_encrypt: true,
    created_at: '2026-08-01T10:00:00Z',
    updated_at: '2026-08-01T10:00:00Z',
    ...overrides,
  };
}

async function renderWithKey(key: BackendPgpKey) {
  backend.backendListPgpKeys.mockResolvedValue([key]);
  const wrapper = mount(PluginManager, { props: { only: 'pgp' as const } });
  await vi.waitFor(() => {
    expect(wrapper.find('.key-card').exists()).toBe(true);
  });
  const deleteButton = wrapper
    .findAll('.key-card-actions button')
    .find((button) => button.text() === 'Delete');
  if (!deleteButton) throw new Error('no delete button');
  return { wrapper, deleteButton };
}

describe('PluginManager PGP key deletion', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    backend.backendDeletePgpKey.mockResolvedValue(undefined);
  });

  it('deletes nothing when the confirmation is declined', async () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    const { wrapper, deleteButton } = await renderWithKey(pgpKey());

    await deleteButton.trigger('click');

    expect(backend.backendDeletePgpKey).not.toHaveBeenCalled();
    expect(wrapper.findAll('.key-card')).toHaveLength(1);
    confirmSpy.mockRestore();
  });

  it('names the fingerprint and the permanent loss before deleting a private key', async () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const { deleteButton } = await renderWithKey(pgpKey());

    await deleteButton.trigger('click');

    const question = confirmSpy.mock.calls[0]?.[0] ?? '';
    expect(question).toContain('AAAA BBBB CCCC DDDD EEEE FFFF 0000 1111 2222 3333');
    expect(question).toContain('can never be opened again');
    confirmSpy.mockRestore();
  });

  it('deletes the confirmed key exactly once', async () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const { deleteButton } = await renderWithKey(pgpKey({ id: 'key-7' }));

    await deleteButton.trigger('click');
    await vi.waitFor(() => {
      expect(backend.backendDeletePgpKey).toHaveBeenCalledTimes(1);
    });

    expect(backend.backendDeletePgpKey).toHaveBeenCalledWith('key-7');
    confirmSpy.mockRestore();
  });

  it('raises a toast when the deletion fails', async () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    backend.backendDeletePgpKey.mockRejectedValue(new Error('keyring locked'));
    const { deleteButton } = await renderWithKey(pgpKey());

    await deleteButton.trigger('click');
    await vi.waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith('Delete failed: keyring locked');
    });
    confirmSpy.mockRestore();
  });
});

describe('PluginManager capability switches', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    backend.backendListPgpKeys.mockResolvedValue([]);
  });

  it('names the capabilities the runtime does not enforce', async () => {
    backend.backendListPlugins.mockResolvedValue([
      {
        ...pgpPlugin,
        id: 'satchel-plugin-github-integration',
        capabilities: ['read_threads', 'network_access'],
        granted_capabilities: ['read_threads'],
        enforced_capabilities: ['read_threads'],
      },
    ]);

    const wrapper = mount(PluginManager, { props: { only: 'plugins' as const } });
    await vi.waitFor(() => {
      expect(wrapper.find('.plugin-card').exists()).toBe(true);
    });

    const text = wrapper.text();
    expect(text).toContain('Not enforced yet: network_access');
    expect(text).not.toContain('Not enforced yet: read_threads');
  });
});

describe('PluginManager on/off switch', () => {
  const integration: BackendPlugin = {
    ...pgpPlugin,
    id: 'satchel-plugin-github-integration',
  };

  beforeEach(() => {
    vi.clearAllMocks();
    backend.backendListPgpKeys.mockResolvedValue([]);
  });

  async function renderPlugin() {
    const wrapper = mount(PluginManager, { props: { only: 'plugins' as const } });
    await vi.waitFor(() => {
      expect(wrapper.find('.plugin-card .plugin-enabled input').exists()).toBe(true);
    });
    return wrapper;
  }

  it('switches a plugin off once and shows the state the backend reports', async () => {
    backend.backendListPlugins
      .mockResolvedValueOnce([integration])
      .mockResolvedValue([{ ...integration, enabled: false, disabled_by: 'user' }]);
    const wrapper = await renderPlugin();

    await wrapper.find('.plugin-card .plugin-enabled input').setValue(false);
    await vi.waitFor(() => {
      expect(
        (wrapper.find('.plugin-card .plugin-enabled input').element as HTMLInputElement).checked,
      ).toBe(false);
    });

    expect(backend.backendSetPluginEnabled).toHaveBeenCalledTimes(1);
    expect(backend.backendSetPluginEnabled).toHaveBeenCalledWith({
      pluginId: 'satchel-plugin-github-integration',
      enabled: false,
    });
  });

  it('says why a plugin was switched off after timeouts', async () => {
    backend.backendListPlugins.mockResolvedValue([
      { ...integration, enabled: false, disabled_by: 'timeouts' },
    ]);
    const wrapper = await renderPlugin();

    expect(wrapper.find('.plugin-card').text()).toContain('Switched off after repeated timeouts');
  });

  it('cannot switch on a plugin whose manifest has errors', async () => {
    backend.backendListPlugins.mockResolvedValue([
      { ...integration, enabled: false, manifest_errors: ['unsupported api version'] },
    ]);
    const wrapper = await renderPlugin();

    const input = wrapper.find('.plugin-card .plugin-enabled input');
    expect((input.element as HTMLInputElement).disabled).toBe(true);
    await input.trigger('change');
    expect(backend.backendSetPluginEnabled).not.toHaveBeenCalled();
  });
});

/** A webhook forwards message metadata to a foreign host, so it must be removable. */
function webhook(overrides: Partial<BackendWebhook> = {}): BackendWebhook {
  return {
    id: 'hook-1',
    url: 'https://example.test/satchel',
    events_json: '["message.received","thread.updated"]',
    enabled: 1,
    created_at: '2026-08-01T10:00:00Z',
    updated_at: '2026-08-01T10:00:00Z',
    ...overrides,
  };
}

async function renderWithWebhook(entry: BackendWebhook = webhook()) {
  backend.backendListPgpKeys.mockResolvedValue([]);
  backend.backendListWebhooks.mockResolvedValue([entry]);
  const wrapper = mount(PluginManager, { props: { only: 'plugins' as const } });
  await vi.waitFor(() => {
    expect(wrapper.find('.webhook-row').exists()).toBe(true);
  });
  return wrapper;
}

describe('PluginManager webhooks', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    backend.backendDeleteWebhook.mockResolvedValue(undefined);
    backend.backendUpsertWebhook.mockResolvedValue(webhook());
  });

  it('renders the subscribed events as chips instead of a JSON string', async () => {
    const wrapper = await renderWithWebhook();

    const chips = wrapper.findAll('.webhook-chip').map((chip) => chip.text());
    expect(chips).toEqual(['message.received', 'thread.updated']);
    expect(wrapper.find('.webhook-row').text()).not.toContain('["message.received"');
  });

  it('keeps the row readable when the stored event list is broken', async () => {
    const wrapper = await renderWithWebhook(webhook({ events_json: 'not json' }));

    expect(wrapper.find('.webhook-url').text()).toBe('https://example.test/satchel');
    expect(wrapper.findAll('.webhook-chip')).toHaveLength(0);
  });

  it('deletes nothing when the confirmation is declined', async () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    const wrapper = await renderWithWebhook();

    await wrapper.find('.webhook-row button').trigger('click');

    expect(backend.backendDeleteWebhook).not.toHaveBeenCalled();
    expect(wrapper.findAll('.webhook-row')).toHaveLength(1);
    confirmSpy.mockRestore();
  });

  it('names the target URL in the question and deletes the confirmed webhook once', async () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const wrapper = await renderWithWebhook(webhook({ id: 'hook-7' }));

    await wrapper.find('.webhook-row button').trigger('click');
    await vi.waitFor(() => {
      expect(backend.backendDeleteWebhook).toHaveBeenCalledTimes(1);
    });

    expect(confirmSpy.mock.calls[0]?.[0] ?? '').toContain('https://example.test/satchel');
    expect(backend.backendDeleteWebhook).toHaveBeenCalledWith('hook-7');
    confirmSpy.mockRestore();
  });

  it('keeps the row and raises a toast when the deletion fails', async () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    backend.backendDeleteWebhook.mockRejectedValue(new Error('database is locked'));
    const wrapper = await renderWithWebhook();

    await wrapper.find('.webhook-row button').trigger('click');
    await vi.waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith('Could not delete webhook: database is locked');
    });

    expect(wrapper.findAll('.webhook-row')).toHaveLength(1);
    confirmSpy.mockRestore();
  });

  it('switches a webhook off through the upsert', async () => {
    const wrapper = await renderWithWebhook();

    await wrapper.find('.webhook-row input[type="checkbox"]').setValue(false);
    await vi.waitFor(() => {
      expect(backend.backendUpsertWebhook).toHaveBeenCalledTimes(1);
    });

    expect(backend.backendUpsertWebhook).toHaveBeenCalledWith({
      id: 'hook-1',
      url: 'https://example.test/satchel',
      events: ['message.received', 'thread.updated'],
      enabled: false,
    });
  });

  it('puts the switch back when the backend refuses', async () => {
    backend.backendUpsertWebhook.mockRejectedValue(new Error('webhook store unavailable'));
    const wrapper = await renderWithWebhook();

    await wrapper.find('.webhook-row input[type="checkbox"]').setValue(false);
    await vi.waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith(
        'Could not switch webhook: webhook store unavailable',
      );
    });

    const toggle = wrapper.find('.webhook-row input[type="checkbox"]')
      .element as HTMLInputElement;
    expect(toggle.checked).toBe(true);
  });
});
