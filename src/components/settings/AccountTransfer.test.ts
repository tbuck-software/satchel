// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { h } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import AccountTransfer from './AccountTransfer.vue';
import ModalShell from '@/components/ui/ModalShell.vue';

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
  it('tabs to the manual disclosure and reaches its address field only when open', async () => {
    setActivePinia(createPinia());
    transfer.mockImplementation(async (action) => action === 'discover' ? { ...idle, phase: 'searching' } : idle);
    const wrapper = mount(ModalShell, {
      attachTo: document.body,
      props: { label: 'Transfer accounts' },
      slots: { default: () => h(AccountTransfer, { receive: false }) },
    });
    try {
      await flushPromises();
      const summary = wrapper.get('summary');
      expect(summary.text()).toBe('Connect by address');
      await wrapper.get('[role="dialog"]').trigger('keydown', { key: 'Tab' });
      expect(document.activeElement).toBe(summary.element);

      await summary.trigger('keydown', { key: 'Tab' });
      expect(document.activeElement).toBe(wrapper.get('button').element);
      await wrapper.get('button').trigger('keydown', { key: 'Tab', shiftKey: true });
      expect(document.activeElement).toBe(summary.element);

      (summary.element as HTMLElement).click();
      expect(wrapper.get('details').element.open).toBe(true);
      await summary.trigger('keydown', { key: 'Tab' });
      expect(document.activeElement).toBe(wrapper.get('details input').element);
    } finally {
      wrapper.unmount();
    }
  });

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
