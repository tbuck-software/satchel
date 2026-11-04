// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAccountsStore } from '@/stores/accounts';
import { useCertificateTrustStore } from '@/stores/certificateTrust';
import { useConnectionStore } from '@/stores/connection';
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
});
