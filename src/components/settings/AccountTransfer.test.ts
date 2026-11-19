// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import AccountTransfer from './AccountTransfer.vue';

const transfer = vi.hoisted(() => vi.fn());
const store = vi.hoisted(() => ({
  emailAccounts: [
    { id: 'imap', email: 'mail@example.test', provider: 'imap' },
    { id: 'google', email: 'google@example.test', provider: 'gmail' },
  ],
  startReauth: vi.fn(),
  load: vi.fn(),
}));
vi.mock('@/services/backend', () => ({ backendAccountTransfer: transfer, translateBackendError: () => null }));
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => store }));

const idle = { phase: 'idle', address: null, code: null, nearby: [], emails: [], sign_in: [], created: 0 };
beforeEach(() => { vi.clearAllMocks(); vi.useFakeTimers(); });
afterEach(() => vi.useRealTimers());

describe('setting up a second device', () => {
  it('sends only explicitly selected IDs after entering the destination code', async () => {
    const nearby = [{ name: 'Desktop', address: '192.0.2.1:1234' }];
    transfer.mockImplementation(async (action) => action === 'discover' ? { ...idle, phase: 'searching', nearby } : idle);
    const wrapper = mount(AccountTransfer, { props: { receive: false } });
    await flushPromises();
    const submit = wrapper.get('button.app-button--primary');
    await wrapper.get('input[inputmode="numeric"]').setValue('012 345');
    expect(submit.attributes('disabled')).toBeDefined();
    await wrapper.findAll('input[type="checkbox"]')[1]!.setValue(true);
    await submit.trigger('click');
    expect(transfer).toHaveBeenCalledWith('send', expect.any(String), {
      address: nearby[0]!.address, code: '012345', accounts: ['google'],
    });
    wrapper.unmount();
  });

  it('waits for explicit import and offers the provider login on the receiving device', async () => {
    transfer.mockImplementation(async (action) => {
      if (action === 'receive') return { ...idle, phase: 'waiting', code: '012345' };
      if (action === 'status') return { ...idle, phase: 'preview', emails: ['google@example.test'] };
      if (action === 'import') return { ...idle, phase: 'imported', created: 1, emails: ['google@example.test'], sign_in: ['google@example.test'] };
      return idle;
    });
    const wrapper = mount(AccountTransfer, { props: { receive: true } });
    await flushPromises();
    expect(wrapper.get('.code').text()).toBe('012 345');
    await vi.advanceTimersByTimeAsync(1000);
    expect(wrapper.text()).toContain('google@example.test');
    expect(transfer.mock.calls.map(([action]) => action)).toEqual(['receive', 'status']);
    await wrapper.get('button.app-button--primary').trigger('click');
    await flushPromises();
    await wrapper.get('button.app-button--soft').trigger('click');
    await flushPromises();
    expect(store.startReauth).toHaveBeenCalledWith('google');
    await wrapper.get('button.app-button--primary').trigger('click');
    expect(wrapper.emitted('imported')).toHaveLength(1);
    wrapper.unmount();
  });
});
