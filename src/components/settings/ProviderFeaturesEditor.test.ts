// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  backendListProviderLabels: vi.fn(),
  backendRefreshProviderLabels: vi.fn(),
  backendCreateProviderLabel: vi.fn(),
}));

vi.mock('@/services/backend', () => backend);
vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => ({
    load: vi.fn(async () => {}),
    activeAccountId: 'acc-1',
    emailAccounts: [
      { id: 'acc-1', email: 'a@example.test', displayName: 'A', provider: 'gmail' },
    ],
  }),
}));

import ProviderFeaturesEditor from './ProviderFeaturesEditor.vue';

function label(name: string) {
  return {
    id: `l-${name}`,
    account_id: 'acc-1',
    provider: 'gmail',
    remote_id: `remote-${name}`,
    name,
    color: null,
    kind: 'user',
    visible: 1,
    updated_at: '2026-09-01T10:00:00Z',
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  backend.backendListProviderLabels.mockResolvedValue([label('Rechnungen')]);
  backend.backendRefreshProviderLabels.mockResolvedValue([]);
});

describe('ProviderFeaturesEditor', () => {
  it('shows the locally stored labels on open without touching the provider', async () => {
    const wrapper = mount(ProviderFeaturesEditor);

    await vi.waitFor(() => expect(wrapper.text()).toContain('Rechnungen'));
    expect(backend.backendListProviderLabels).toHaveBeenCalledWith('acc-1');
    // Refresh is the only path that may reach the provider.
    expect(backend.backendRefreshProviderLabels).not.toHaveBeenCalled();
  });

  it('keeps the pane usable when the local read fails', async () => {
    backend.backendListProviderLabels.mockRejectedValue(new Error('database is locked'));
    const wrapper = mount(ProviderFeaturesEditor);

    await vi.waitFor(() => expect(wrapper.get('.status').text()).toContain('database is locked'));
  });
});
