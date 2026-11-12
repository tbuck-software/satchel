// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import type { Account } from '@/types/mail';
import ShellStatusChip from './ShellStatusChip.vue';

// The store only records a failure when it believes there is a real backend.
type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

describe('ShellStatusChip', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (window as TauriWindow).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete (window as TauriWindow).__TAURI_INTERNALS__;
  });

  it('says nothing while all is well', () => {
    expect(mount(ShellStatusChip).find('.shell-status').exists()).toBe(false);
  });

  it('tells offline and syncing as plain status, not as a button', () => {
    const connection = useConnectionStore();
    connection.online = false;
    const wrapper = mount(ShellStatusChip);

    expect(wrapper.get('[role="status"]').text()).toBe('Offline');
    expect(wrapper.find('button').exists()).toBe(false);

    connection.online = true;
    connection.syncing = true;
    return wrapper.vm.$nextTick().then(() => {
      expect(wrapper.get('[role="status"]').text()).toBe('Syncing');
    });
  });

  it('names the weightiest problem, counts the rest, and opens its details ahead of import progress', async () => {
    const connection = useConnectionStore();
    connection.reportBackendError('sync', new Error('host unreachable'));
    connection.online = false;
    connection.syncProgress = [{ account_id: 'a', account_name: 'Work', email: 'a@example.test',
      phase: 'waiting', folder: null, folder_index: null, folder_total: null,
      backfill_remaining: 20, backfill_total: 40, updated_at: new Date().toISOString() }];
    useAccountsStore().accounts = [{
      id: 'a', email: 'a@example.test', provider: 'google', displayName: 'a@example.test',
      kind: 'email', color: '#000000', lastOauthError: 'invalid_grant',
    } as Account];
    const wrapper = mount(ShellStatusChip, { global: { stubs: {
      ShellStatusSheet: { template: '<div data-testid="status-details" />' },
      SyncProgressDetails: { template: '<div data-testid="import-details" />' },
    } } });

    const chip = wrapper.get('button');
    expect(chip.text()).toContain('Sign-in needed');
    // The sync failure is the other issue; offline is a state, not counted.
    expect(chip.get('.shell-status__more').text()).toBe('+1');
    expect(chip.attributes('aria-haspopup')).toBe('dialog');
    expect(chip.classes()).toContain('shell-status__chip--danger');
    await chip.trigger('click');
    await flushPromises();
    expect(wrapper.find('[data-testid="status-details"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="import-details"]').exists()).toBe(false);
  });
});
