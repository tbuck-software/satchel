// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAccountsStore } from '@/stores/accounts';
import { useCertificateTrustStore } from '@/stores/certificateTrust';
import { useConnectionStore } from '@/stores/connection';
import { useAccountQuotaStore } from '@/stores/accountQuota';
import type { BackendAccountQuota } from '@/services/backend';
import type { Account } from '@/types/mail';

const router = vi.hoisted(() => ({ push: vi.fn() }));
vi.mock('vue-router', () => ({ useRouter: () => router }));

import SidebarAccounts from './SidebarAccounts.vue';

function account(id: string, displayName: string): Account {
  return { id, email: `${id}@example.test`, provider: 'imap', displayName, kind: 'email', color: '#000000' } as Account;
}

function failure(accountId: string, message: string, since: string): string {
  return JSON.stringify({
    key: 'backend.errors.accountSyncFailed',
    params: { account: `${accountId}@example.test`, accountId, message, since },
  });
}

const rejection = JSON.stringify({
  key: 'backend.errors.tlsCertificateRejected',
  params: { protocol: 'imap', host: 'imap.example.test', port: '993', reason: 'expired', fingerprint: 'AA', exceptionAllowed: true, notAfter: '2026-11-04T11:06:26Z' },
});

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

describe('account markers in the sidebar', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (window as TauriWindow).__TAURI_INTERNALS__ = {};
    router.push.mockClear();
    useAccountsStore().accounts = [account('acc-work', 'Work'), account('acc-home', 'Home')];
  });

  afterEach(() => {
    delete (window as TauriWindow).__TAURI_INTERNALS__;
  });

  it('marks an account that cannot sync, says since when, and opens its settings', async () => {
    useConnectionStore().reportSyncFailures([failure('acc-work', 'imap tcp connect timed out', '2026-10-14T11:00:00Z')]);
    const wrapper = mount(SidebarAccounts);

    expect(wrapper.findAll('.sidebar-accounts__certificate')).toHaveLength(1);
    const marker = wrapper.get('.sidebar-accounts__certificate');
    expect(marker.attributes('title')).toMatch(/^Work: no sync since /);

    await marker.trigger('click');
    expect(router.push).toHaveBeenCalledWith({ name: 'settings', query: { section: 'accounts', account: 'acc-work' } });
  });

  it('opens the certificate details for a rejected certificate instead', async () => {
    useConnectionStore().reportSyncFailures([failure('acc-work', rejection, '2026-11-04T11:10:00Z')]);
    const wrapper = mount(SidebarAccounts);

    const marker = wrapper.get('.sidebar-accounts__certificate');
    expect(marker.attributes('title')).toMatch(/^Work: Certificate expired since /);
    await marker.trigger('click');

    expect(useCertificateTrustStore().detailsAccountId).toBe('acc-work');
    expect(router.push).not.toHaveBeenCalled();
  });

  it('shows nothing while every account syncs', () => {
    const wrapper = mount(SidebarAccounts);

    expect(wrapper.find('.sidebar-accounts__certificate').exists()).toBe(false);
  });

  function quotaAt(accountId: string, used: number): BackendAccountQuota {
    return { account_id: accountId, storage_used_bytes: used, storage_limit_bytes: 1000, messages_used: null, messages_limit: null, fetched_at: '2026-11-05T08:00:00Z' };
  }

  it('stays quiet below 90 % full', () => {
    useAccountQuotaStore().quotas = [quotaAt('acc-work', 899)];
    const wrapper = mount(SidebarAccounts);

    expect(wrapper.find('.sidebar-accounts__certificate').exists()).toBe(false);
  });

  it('marks a mailbox from 90 % and sends the user to the account settings', async () => {
    useAccountQuotaStore().quotas = [quotaAt('acc-work', 930)];
    const wrapper = mount(SidebarAccounts);

    const marker = wrapper.get('.sidebar-accounts__certificate');
    expect(marker.classes()).toContain('sidebar-accounts__quota');
    expect(marker.classes()).not.toContain('sidebar-accounts__quota--critical');
    expect(marker.attributes('title')).toBe('Work: mailbox 93% full. Open account settings.');
    await marker.trigger('click');
    // No `account`: on the accounts page that opens the password form.
    expect(router.push).toHaveBeenCalledWith({ name: 'settings', query: { section: 'accounts' } });
  });

  it('turns critical from 98 %', () => {
    useAccountQuotaStore().quotas = [quotaAt('acc-home', 985)];
    const wrapper = mount(SidebarAccounts);

    const marker = wrapper.get('.sidebar-accounts__certificate');
    expect(marker.classes()).toContain('sidebar-accounts__quota--critical');
    expect(marker.attributes('title')).toMatch(/^Home: mailbox 98% full, new mail may bounce/);
  });

  it('lets a sync outage outrank a filling mailbox', () => {
    useAccountQuotaStore().quotas = [quotaAt('acc-work', 990)];
    useConnectionStore().reportSyncFailures([failure('acc-work', 'imap tcp connect timed out', '2026-10-14T11:00:00Z')]);
    const wrapper = mount(SidebarAccounts);

    expect(wrapper.get('.sidebar-accounts__certificate').attributes('title')).toMatch(/^Work: no sync since /);
  });

  it('shows a lone account once its mailbox fills up, and only then', async () => {
    useAccountsStore().accounts = [account('acc-work', 'Work')];
    const wrapper = mount(SidebarAccounts);
    expect(wrapper.find('nav').exists()).toBe(false);

    useAccountQuotaStore().quotas = [quotaAt('acc-work', 950)];
    await wrapper.vm.$nextTick();
    expect(wrapper.find('nav').exists()).toBe(true);
    expect(wrapper.find('.sidebar-accounts__quota').exists()).toBe(true);
  });
});
