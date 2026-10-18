// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useAccountsStore } from '@/stores/accounts';
import { backendOauthStatus } from '@/services/backend';
import GoogleConnections from './GoogleConnections.vue';

vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  backendOauthStatus: vi.fn(),
}));

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  useAccountsStore().accounts = [{ id: 'gmail', email: 'person@example.test',
    displayName: 'Personal', provider: 'gmail', kind: 'email', color: 'blue' }];
  vi.mocked(backendOauthStatus).mockResolvedValue({ account_id: 'gmail', provider: 'gmail',
    connected: true, has_refresh_token: true, expires_at: null,
    granted_scopes: ['https://www.googleapis.com/auth/gmail.modify'] });
});

describe('Google feature connections', () => {
  it('asks for calendar access only after choosing Connect', async () => {
    const accounts = useAccountsStore();
    const reconnect = vi.spyOn(accounts, 'startReauth').mockImplementation(async () => {
      vi.mocked(backendOauthStatus).mockResolvedValue({ account_id: 'gmail', provider: 'gmail',
        connected: true, has_refresh_token: true, expires_at: null,
        granted_scopes: ['https://www.googleapis.com/auth/gmail.modify', 'https://www.googleapis.com/auth/calendar.readonly'] });
    });
    const wrapper = mount(GoogleConnections, { props: { purpose: 'calendar' } });
    await flushPromises();
    expect(reconnect).not.toHaveBeenCalled();
    expect(wrapper.get('button').text()).toContain('Connect');
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(reconnect).toHaveBeenCalledWith('gmail', 'calendar');
    expect(wrapper.emitted('connected')).toEqual([['gmail']]);
    expect(wrapper.get('button').attributes('disabled')).toBeDefined();
  });

  it('keeps declined contact access disconnected and displays the reason', async () => {
    const reconnect = vi.spyOn(useAccountsStore(), 'startReauth').mockRejectedValue(
      new Error('Contact access was declined'),
    );
    const wrapper = mount(GoogleConnections, { props: { purpose: 'contacts' } });
    await flushPromises();
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(reconnect).toHaveBeenCalledWith('gmail', 'contacts');
    expect(wrapper.emitted('connected')).toBeUndefined();
    expect(wrapper.get('[role="alert"]').text()).toContain('Contact access was declined');
    expect(wrapper.get('button').attributes('disabled')).toBeUndefined();
  });
});
