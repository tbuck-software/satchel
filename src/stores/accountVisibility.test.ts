// @vitest-environment jsdom
import { nextTick } from 'vue';
import { createPinia, disposePinia, setActivePinia, type Pinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendAccountVisibilityStatus } from '@/services/backend';

const backend = vi.hoisted(() => ({ get: vi.fn(), set: vi.fn(), topics: vi.fn() }));
vi.mock('@/services/backend', async importOriginal => ({
  ...await importOriginal<typeof import('@/services/backend')>(),
  backendCommandsAvailable: () => true,
  backendGetAccountVisibility: backend.get,
  backendSetAccountVisibility: backend.set,
  backendListTopics: backend.topics,
  backendListAccounts: async () => [{ id: 'work', display_name: 'Work', email: 'work@example.test', kind: 'email', provider: 'imap' }],
  backendGetSourceCapabilities: async () => ({}),
  backendCredentialsStatus: async () => ({ master_key_present: true, plaintext_accounts: 0 }),
}));
import { useAccountsStore } from './accounts';
import { useTopicsStore, type Topic } from './topics';

let pinia: Pinia;
const status = (visible: boolean): BackendAccountVisibilityStatus[] => [{
  account_id: 'work', visible, local_date: '2026-10-26',
  policy: { mode: 'schedule', weekdays: [1], from: '09:00', to: '17:00', override_date: null, override_visible: null },
}];

beforeEach(() => {
  vi.useFakeTimers();
  pinia = createPinia(); setActivePinia(pinia);
  backend.get.mockReset().mockResolvedValue(status(true));
  backend.set.mockReset().mockResolvedValue(undefined);
  backend.topics.mockReset().mockResolvedValue([]);
});
afterEach(() => { disposePinia(pinia); vi.useRealTimers(); });

describe('account visibility refresh', () => {
  it('does not automatically open the only resting account', async () => {
    backend.get.mockResolvedValue(status(false));
    const accounts = useAccountsStore(); await accounts.load();
    expect(accounts.activeAccountId).toBe('');
    expect(accounts.isVisible('work')).toBe(false);
  });
  it('leaves an automatic account scope when its window ends, but preserves explicit access', async () => {
    const accounts = useAccountsStore(); await accounts.load();
    expect(accounts.activeAccountId).toBe('work');
    backend.get.mockResolvedValue(status(false));
    await vi.advanceTimersByTimeAsync(15_000);
    expect(accounts.activeAccountId).toBe('');
    accounts.setActiveAccount('work');
    await vi.advanceTimersByTimeAsync(15_000);
    expect(accounts.activeAccountId).toBe('work');
    expect(accounts.isVisible('work')).toBe(false);
  });
  it('discards collection counts from a read started before an account rested', async () => {
    let finish!: (rows: Topic[]) => void;
    backend.topics.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const accounts = useAccountsStore();
    const topics = useTopicsStore();
    const pending = topics.load();
    accounts.visibility = status(false);
    await nextTick();
    finish([{
      id: 'work-topic', name: 'Work', description: '', color: null, icon: null,
      status: 'active', due_date: null, item_count: 1, note_count: 0, unread_count: 1,
      last_sender_name: null, last_sender_email: null, created_at: '', updated_at: '',
    }]);
    await pending;
    expect(backend.topics).toHaveBeenCalledTimes(2);
    expect(topics.items).toEqual([]);
  });

  it('preserves an explicitly chosen unified view through account refresh', async () => {
    const accounts = useAccountsStore(); await accounts.load();
    accounts.setActiveAccount('');
    await accounts.load();
    expect(accounts.activeAccountId).toBe('');
    backend.get.mockResolvedValue(status(false));
    await vi.advanceTimersByTimeAsync(15_000);
    expect(accounts.activeAccountId).toBe('');
  });

  it('sets a peek using the backend local calendar date', async () => {
    backend.get.mockResolvedValue(status(false));
    const accounts = useAccountsStore(); await accounts.load();
    await accounts.showToday('work', true);
    expect(backend.set).toHaveBeenCalledWith('work', {
      ...status(false)[0]!.policy, override_date: '2026-10-26', override_visible: true,
    });
  });
});
