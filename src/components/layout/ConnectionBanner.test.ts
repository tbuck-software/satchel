// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import type { Account } from '@/types/mail';

const router = vi.hoisted(() => ({ push: vi.fn() }));
vi.mock('vue-router', () => ({ useRouter: () => router }));

import ConnectionBanner from './ConnectionBanner.vue';

function syncFailure(accountId: string, account: string, message: string): string {
  return JSON.stringify({
    key: 'backend.errors.accountSyncFailed',
    params: { account, accountId, message },
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

    it('offers no password for an account that signs in through OAuth', () => {
      useConnectionStore().reportSyncFailures([syncFailure('acc-gmail', 'tom@gmail.example', '401 Unauthorized')]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.text()).not.toContain('Update password');
      expect(wrapper.text()).toContain('Try again');
    });

    it('keeps other sync failures as they were', () => {
      useConnectionStore().reportSyncFailures([syncFailure('acc-work', 'tom@example.test', 'imap tcp connect timed out')]);
      const wrapper = mount(ConnectionBanner);

      expect(wrapper.get('.banner__detail').text()).toContain('Sync for tom@example.test failed');
      expect(wrapper.find('details').exists()).toBe(false);
      expect(wrapper.text()).not.toContain('Update password');
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
});
