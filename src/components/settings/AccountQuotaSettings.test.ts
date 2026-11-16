// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createMemoryHistory, createRouter } from 'vue-router';
import { applyLocalePreference } from '@/i18n/runtime';
import { useAccountQuotaStore } from '@/stores/accountQuota';
import { useUiStore } from '@/stores/ui';
import type { BackendAccountQuota } from '@/services/backend';
import AccountQuotaSettings from './AccountQuotaSettings.vue';

const plain = (text: string) => text.replace(/[  ]/g, ' ');

function quota(change: Partial<BackendAccountQuota>): BackendAccountQuota {
  return {
    account_id: 'acc-work', storage_used_bytes: null, storage_limit_bytes: null,
    messages_used: null, messages_limit: null, fetched_at: '2026-11-04T08:00:00Z', ...change,
  };
}

describe('the quota on the account card', () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(async () => { await applyLocalePreference('en', false); });

  it('says how much of the mailbox is used, with a meter', () => {
    useAccountQuotaStore().quotas = [quota({ storage_used_bytes: 1_250_000_000, storage_limit_bytes: 2_000_000_000 })];
    const wrapper = mount(AccountQuotaSettings, { props: { accountId: 'acc-work' } });

    expect(plain(wrapper.text())).toContain('1.3 GB of 2 GB used');
    const meter = wrapper.get('[role="meter"]');
    expect(meter.attributes('aria-label')).toBe('Mailbox storage used');
    expect(meter.attributes('aria-valuenow')).toBe('1250000000');
    expect(meter.attributes('aria-valuemax')).toBe('2000000000');
    expect(meter.classes()).toContain('usage-meter--neutral');
    expect(wrapper.find('.quota-settings__note').exists()).toBe(false);
  });

  it('warns at 90 % and turns red at 98 %, with a way to make room', async () => {
    const store = useAccountQuotaStore();
    store.quotas = [quota({ storage_used_bytes: 91, storage_limit_bytes: 100 })];
    const wrapper = mount(AccountQuotaSettings, { props: { accountId: 'acc-work' } });
    expect(wrapper.get('[role="meter"]').classes()).toContain('usage-meter--warning');
    expect(wrapper.get('.quota-settings__note').text()).toMatch(/^Almost full.*Deleting large messages/);

    store.quotas = [quota({ storage_used_bytes: 99, storage_limit_bytes: 100 })];
    await wrapper.vm.$nextTick();
    expect(wrapper.get('[role="meter"]').classes()).toContain('usage-meter--danger');
    expect(wrapper.get('.quota-settings__note').classes()).toContain('quota-settings__note--critical');
  });

  it('offers the cleanup assistant for this account once the mailbox is nearly full', async () => {
    const store = useAccountQuotaStore();
    store.quotas = [quota({ storage_used_bytes: 50, storage_limit_bytes: 100 })];
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/', name: 'home', component: { template: '<div />' } },
        { path: '/cleanup', name: 'cleanup', component: { template: '<div />' } },
      ],
    });
    const push = vi.spyOn(router, 'push');
    const closeSettings = vi.spyOn(useUiStore(), 'closeSettings');
    const wrapper = mount(AccountQuotaSettings, { props: { accountId: 'acc-work' }, global: { plugins: [router] } });
    expect(wrapper.find('.quota-settings__cleanup').exists()).toBe(false);

    store.quotas = [quota({ storage_used_bytes: 95, storage_limit_bytes: 100 })];
    await wrapper.vm.$nextTick();
    await wrapper.get('.quota-settings__cleanup').trigger('click');

    expect(closeSettings).toHaveBeenCalled();
    expect(push).toHaveBeenCalledWith({ name: 'cleanup', query: { account: 'acc-work' } });
  });

  it('shows a message limit next to storage when the server sets both', () => {
    useAccountQuotaStore().quotas = [quota({ storage_used_bytes: 1, storage_limit_bytes: 100, messages_used: 4200, messages_limit: 10_000 })];
    const wrapper = mount(AccountQuotaSettings, { props: { accountId: 'acc-work' } });

    expect(wrapper.findAll('[role="meter"]')).toHaveLength(2);
    expect(wrapper.get('[data-quota-kind="messages"]').text()).toContain('4,200 of 10,000 messages');
  });

  it('is absent for an account without a known quota', () => {
    useAccountQuotaStore().quotas = [quota({ account_id: 'other', storage_used_bytes: 1, storage_limit_bytes: 2 })];
    const wrapper = mount(AccountQuotaSettings, { props: { accountId: 'acc-work' } });

    expect(wrapper.find('section').exists()).toBe(false);
  });

  it('formats sizes and percent the German way', async () => {
    await applyLocalePreference('de', false);
    useAccountQuotaStore().quotas = [quota({ storage_used_bytes: 1_250_000_000, storage_limit_bytes: 2_000_000_000 })];
    const wrapper = mount(AccountQuotaSettings, { props: { accountId: 'acc-work' } });

    expect(plain(wrapper.text())).toContain('1,3 GB von 2 GB belegt');
    expect(plain(wrapper.text())).toContain('62 %');
  });
});
