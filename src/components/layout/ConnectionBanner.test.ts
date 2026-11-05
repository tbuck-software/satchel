// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { useUiStore } from '@/stores/ui';
import type { Account } from '@/types/mail';

const router = vi.hoisted(() => ({ push: vi.fn() }));
vi.mock('vue-router', () => ({ useRouter: () => router }));

import ConnectionBanner from './ConnectionBanner.vue';

function syncFailure(accountId: string, account: string, message: string, since?: string): string {
  return JSON.stringify({
    key: 'backend.errors.accountSyncFailed',
    params: { account, accountId, message, ...(since ? { since } : {}) },
  });
}

function account(id: string, email: string, provider: string): Account {
  return { id, email, provider, displayName: email, kind: 'email', color: '#000000' } as Account;
}

// The store only records a failure when it believes it is talking to a real
// backend; a browser preview has no backend to blame.
type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

describe('ConnectionBanner', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (window as TauriWindow).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete (window as TauriWindow).__TAURI_INTERNALS__;
  });

  it('stays out of the way while everything works', () => {
    const wrapper = mount(ConnectionBanner);

    expect(wrapper.find('.banner').exists()).toBe(false);
  });

  it('names the failing subsystem and the cause, and announces both', async () => {
    const connection = useConnectionStore();
    connection.reportBackendError('threads', new Error('db is locked'));
    const wrapper = mount(ConnectionBanner);

    const banner = wrapper.get('[role="alert"]');
    expect(banner.text()).toContain('The conversations could not be loaded');
    expect(banner.text()).toContain('db is locked');
  });

  it('asks the app to load again', async () => {
    const connection = useConnectionStore();
    connection.reportBackendError('sync', new Error('host unreachable'));
    const wrapper = mount(ConnectionBanner);

    await wrapper.get('button').trigger('click');

    expect(connection.retryToken).toBe(1);
  });

  it('disappears once the reporting subsystem recovers', async () => {
    const connection = useConnectionStore();
    connection.reportBackendError('threads', new Error('db is locked'));
    const wrapper = mount(ConnectionBanner);

    connection.clearBackendError('threads');
    await wrapper.vm.$nextTick();

    expect(wrapper.find('.banner').exists()).toBe(false);
  });

  // #159: the German UI showed "…fehlgeschlagen: NO [AUTHENTICATIONFAILED]" and offered only Retry.
  describe('a rejected password', () => {
    const serverText = 'imap login failed: No Response: [AUTHENTICATIONFAILED] Invalid credentials (Failure)';

    beforeEach(() => {
      router.push.mockClear();
      useAccountsStore().accounts = [
        account('acc-work', 'tom@example.test', 'imap'),
        account('acc-gmail', 'tom@gmail.example', 'gmail'),
      ];
    });

    it('says so, keeps the server text aside, and leads to the account settings', async () => {
      useConnectionStore().reportSyncFailures([syncFailure('acc-work', 'tom@example.test', serverText)]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.get('.banner__detail').text()).toBe('tom@example.test did not accept the password.');
      expect(wrapper.get('details summary').text()).toBe('Server response');
      expect(wrapper.get('details code').text()).toBe(serverText);

      const update = wrapper.findAll('button').find((button) => button.text() === 'Update password');
      expect(update).toBeDefined();
      await update!.trigger('click');
      expect(router.push).toHaveBeenCalledWith({
        name: 'settings',
        query: { section: 'accounts', account: 'acc-work' },
      });
    });

    it('signs an OAuth account in again instead of asking for a password', async () => {
      const accounts = useAccountsStore();
      const startReauth = vi.spyOn(accounts, 'startReauth').mockResolvedValue(undefined);
      useConnectionStore().reportSyncFailures([syncFailure('acc-gmail', 'tom@gmail.example', '401 Unauthorized')]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.text()).not.toContain('Update password');
      expect(wrapper.text()).toContain('Sync again');
      const signIn = wrapper.findAll('button').find((button) => button.text() === 'Sign in again');
      expect(signIn).toBeDefined();
      await signIn!.trigger('click');
      expect(startReauth).toHaveBeenCalledWith('acc-gmail');
    });

    it('keeps other sync failures as they were', () => {
      useConnectionStore().reportSyncFailures([syncFailure('acc-work', 'tom@example.test', 'imap tcp connect timed out')]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.get('.banner__detail').text()).toContain('Sync for tom@example.test failed');
      expect(wrapper.find('details').exists()).toBe(false);
      expect(wrapper.text()).not.toContain('Update password');
    });

    // Owner report: "Der Ordner X … konnte nicht synchronisiert werden" offered a bare "Try again".
    it('syncs a failed folder again and says so', async () => {
      const connection = useConnectionStore();
      connection.reportSyncFailures([JSON.stringify({
        key: 'backend.errors.folderSyncFailed',
        params: { account: 'tom@example.test', folder: 'Abuse', message: 'NO [SERVERBUG] internal error' },
      })]);
      const wrapper = mount(ConnectionBanner);

      const buttons = wrapper.findAll('button').map((button) => button.text());
      expect(buttons).toEqual(['Sync again']);
      await wrapper.get('button').trigger('click');
      expect(connection.retryToken).toBe(1);
    });

    it('opens the account settings when the server name does not resolve', async () => {
      useConnectionStore().reportSyncFailures([syncFailure(
        'acc-work',
        'tom@example.test',
        'failed to lookup address information: nodename nor servname provided',
      )]);
      const wrapper = mount(ConnectionBanner);

      const settings = wrapper.findAll('button').find((button) => button.text() === 'Account settings');
      expect(settings).toBeDefined();
      await settings!.trigger('click');
      expect(router.push).toHaveBeenCalledWith({
        name: 'settings',
        query: { section: 'accounts', account: 'acc-work' },
      });
      expect(wrapper.text()).toContain('Sync again');
    });

    it('goes away when the next run comes back clean', async () => {
      const connection = useConnectionStore();
      connection.reportSyncFailures([syncFailure('acc-work', 'tom@example.test', serverText)]);
      const wrapper = mount(ConnectionBanner);

      connection.reportSyncFailures([]);
      await wrapper.vm.$nextTick();

      expect(wrapper.find('.banner').exists()).toBe(false);
      expect(connection.syncFailures).toEqual([]);
    });
  });
  // One IMAP node behind a load balancer presented a certificate that had expired that morning;
  // the banner showed a cut-off rustls error chain in red over the whole app.
  describe('a rejected server certificate', () => {
    const rejection = (fingerprint = '6D:39:A7:FC') => JSON.stringify({
      key: 'backend.errors.tlsCertificateRejected',
      params: {
        protocol: 'imap',
        host: 'gwimap.example.test',
        port: '993',
        reason: 'expired',
        subject: 'gwimap-02.example.test',
        issuer: 'GEANT TLS RSA 1',
        notBefore: '2025-11-04T11:06:26Z',
        notAfter: '2026-11-04T11:06:26Z',
        fingerprint,
        exceptionAllowed: true,
      },
    });

    beforeEach(() => {
      window.localStorage.clear();
      useAccountsStore().accounts = [
        { ...account('acc-work', 'tom@example.test', 'imap'), displayName: 'Work' },
        account('acc-other', 'tom@other.example', 'imap'),
      ];
    });

    it('names the account, the server and who has to act, in the warning tone', () => {
      useConnectionStore().reportSyncFailures([syncFailure('acc-work', 'tom@example.test', rejection())]);
      const wrapper = mount(ConnectionBanner);

      const banner = wrapper.get('.banner');
      expect(banner.classes()).toContain('banner--warning');
      expect(banner.attributes('role')).toBe('status');
      expect(wrapper.get('.banner__title').text()).toBe("Work: The mail server's certificate has expired");
      expect(wrapper.get('.banner__detail').text()).toContain('gwimap.example.test has presented an expired certificate since');
      expect(wrapper.text()).not.toContain('handshake');
    });

    it('stays hidden for that certificate once hidden, also after a restart', async () => {
      const connection = useConnectionStore();
      connection.reportSyncFailures([syncFailure('acc-work', 'tom@example.test', rejection())]);
      const wrapper = mount(ConnectionBanner);

      await wrapper.findAll('button').find((button) => button.text() === 'Hide')!.trigger('click');
      expect(wrapper.find('.banner').exists()).toBe(false);

      // The next run fails the same way: still nothing to see.
      connection.reportSyncFailures([syncFailure('acc-work', 'tom@example.test', rejection())]);
      await wrapper.vm.$nextTick();
      expect(wrapper.find('.banner').exists()).toBe(false);

      // A fresh app with the same certificate rejected.
      setActivePinia(createPinia());
      useAccountsStore().accounts = [{ ...account('acc-work', 'tom@example.test', 'imap'), displayName: 'Work' }];
      useConnectionStore().reportSyncFailures([syncFailure('acc-work', 'tom@example.test', rejection())]);
      expect(mount(ConnectionBanner).find('.banner').exists()).toBe(false);
    });

    it('comes back for a different certificate', async () => {
      const connection = useConnectionStore();
      connection.reportSyncFailures([syncFailure('acc-work', 'tom@example.test', rejection())]);
      const wrapper = mount(ConnectionBanner);
      await wrapper.findAll('button').find((button) => button.text() === 'Hide')!.trigger('click');

      connection.reportSyncFailures([syncFailure('acc-work', 'tom@example.test', rejection('AA:BB:CC'))]);
      await wrapper.vm.$nextTick();

      expect(wrapper.find('.banner--warning').exists()).toBe(true);
    });

    it('leaves other failures to the danger banner, without the certificate', () => {
      useConnectionStore().reportSyncFailures([
        syncFailure('acc-work', 'tom@example.test', rejection()),
        syncFailure('acc-other', 'tom@other.example', 'imap tcp connect timed out'),
      ]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.findAll('.banner')).toHaveLength(1);
      expect(wrapper.get('.banner').classes()).toContain('banner--danger');
      expect(wrapper.get('.banner__detail').text()).toContain('tom@other.example');
      expect(wrapper.get('.banner__detail').text()).not.toContain('gwimap');
    });
  });
  // October: one account did not sync for two days, and the banner read the
  // same as a hiccup a minute old.
  describe('an account that has not synced for a while', () => {
    const hoursAgo = (hours: number) => new Date(Date.now() - hours * 3_600_000).toISOString();

    beforeEach(() => {
      router.push.mockClear();
      useAccountsStore().accounts = [
        { ...account('acc-work', 'tom@example.test', 'imap'), displayName: 'Work' },
        account('acc-other', 'tom@other.example', 'imap'),
      ];
    });

    it('says since when, and leads to the account', async () => {
      useConnectionStore().reportSyncFailures([
        syncFailure('acc-work', 'tom@example.test', 'imap tcp connect timed out', hoursAgo(30)),
      ]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.get('.banner__title').text()).toMatch(/^Work has not synced since /);
      expect(wrapper.get('.banner__detail').text()).toContain('imap tcp connect timed out');
      await wrapper.findAll('button').find((button) => button.text() === 'Account settings')!.trigger('click');
      expect(router.push).toHaveBeenCalledWith({
        name: 'settings',
        query: { section: 'accounts', account: 'acc-work' },
      });
    });

    it('keeps the plain headline for a failure that just began', () => {
      useConnectionStore().reportSyncFailures([
        syncFailure('acc-work', 'tom@example.test', 'imap tcp connect timed out', hoursAgo(0.1)),
      ]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.get('.banner__title').text()).toBe('Mail sync failed');
      expect(wrapper.text()).not.toContain('Account settings');
    });

    it('keeps the plain headline when several accounts fail at once', () => {
      useConnectionStore().reportSyncFailures([
        syncFailure('acc-work', 'tom@example.test', 'offline', hoursAgo(3)),
        syncFailure('acc-other', 'tom@other.example', 'offline', hoursAgo(3)),
      ]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.get('.banner__title').text()).toBe('Mail sync failed');
    });

    it('puts a dot on the drawer handle for an outage, not for a hiccup', async () => {
      const connection = useConnectionStore();
      const ui = useUiStore();
      connection.reportSyncFailures([syncFailure('acc-work', 'tom@example.test', 'offline', hoursAgo(0.1))]);
      const wrapper = mount(ConnectionBanner);
      expect(ui.drawerAttention).toBe(false);

      connection.reportSyncFailures([syncFailure('acc-work', 'tom@example.test', 'offline', hoursAgo(2))]);
      await wrapper.vm.$nextTick();
      expect(ui.drawerAttention).toBe(true);

      connection.reportSyncFailures([]);
      await wrapper.vm.$nextTick();
      expect(ui.drawerAttention).toBe(false);
    });

    // A blocked folder reports a bare entry of its own before the account's.
    it('still names the account when a blocked folder reports alongside it', () => {
      const blocked = JSON.stringify({
        key: 'backend.errors.folderIngestBlocked',
        params: { account: 'tom@example.test', folders: 'INBOX', count: 3 },
      });
      useConnectionStore().reportSyncFailures([
        blocked,
        syncFailure('acc-work', 'tom@example.test', blocked, hoursAgo(48)),
      ]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.get('.banner__title').text()).toMatch(/^Work has not synced since /);
    });

    it('turns the headline at the hour without a new report', async () => {
      vi.useFakeTimers();
      try {
        useConnectionStore().reportSyncFailures([
          syncFailure('acc-work', 'tom@example.test', 'offline', new Date(Date.now() - 59 * 60_000).toISOString()),
        ]);
        const wrapper = mount(ConnectionBanner);
        expect(wrapper.get('.banner__title').text()).toBe('Mail sync failed');

        await vi.advanceTimersByTimeAsync(2 * 60_000);

        expect(wrapper.get('.banner__title').text()).toMatch(/^Work has not synced since /);
      } finally {
        vi.useRealTimers();
      }
    });
  });
});
