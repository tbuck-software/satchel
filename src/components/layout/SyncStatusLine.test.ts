// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useConnectionStore } from '@/stores/connection';
import SyncStatusLine from './SyncStatusLine.vue';

// Same reason as ConnectionBanner.test.ts: the store only records a failure
// when it believes there is a real backend to blame.
type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

describe('SyncStatusLine', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    (window as TauriWindow).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete (window as TauriWindow).__TAURI_INTERNALS__;
  });

  it('says we are offline, as a state rather than an alert', () => {
    const connection = useConnectionStore();
    connection.online = false;

    const wrapper = mount(SyncStatusLine);

    expect(wrapper.get('[role="status"]').text()).toContain('Offline — no connection');
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  });

  it('reports a running sync', () => {
    const connection = useConnectionStore();
    connection.syncing = true;

    expect(mount(SyncStatusLine).get('[role="status"]').text()).toContain('Syncing…');
  });

  it('shows the last sync as a formatted time, not a raw timestamp', () => {
    const connection = useConnectionStore();
    connection.lastSyncAt = new Date().toISOString();

    const text = mount(SyncStatusLine).get('[role="status"]').text();

    expect(text).toContain('Last synced');
    expect(text).not.toContain(connection.lastSyncAt);
  });

  it('admits that nothing has synced yet', () => {
    useConnectionStore();

    expect(mount(SyncStatusLine).get('[role="status"]').text()).toContain('Not synced yet');
  });

  it('leaves a standing backend failure to the banner', () => {
    const connection = useConnectionStore();
    connection.reportBackendError('threads', new Error('db is locked'));

    expect(mount(SyncStatusLine).find('[role="status"]').exists()).toBe(false);
  });

  it('keeps quiet on the phone until there is something to say', async () => {
    const connection = useConnectionStore();
    connection.lastSyncAt = new Date().toISOString();
    const wrapper = mount(SyncStatusLine, { props: { hideIdle: true } });

    expect(wrapper.find('[role="status"]').exists()).toBe(false);

    connection.online = false;
    await wrapper.vm.$nextTick();

    expect(wrapper.get('[role="status"]').text()).toContain('Offline — no connection');
  });
});

it('shows initial mail import progress instead of only a generic busy label', () => {
  setActivePinia(createPinia());
  const connection = useConnectionStore();
  connection.syncing = true;
  connection.syncProgress = [{ account_id: 'account', account_name: 'Work', email: 'me@example.test',
    phase: 'folder', folder: 'Archive', folder_index: 2, folder_total: 4,
    backfill_total: 1000, backfill_remaining: 200, updated_at: '2026-11-12T12:00:00Z' }];
  const wrapper = mount(SyncStatusLine, { props: { hideIdle: true } });
  expect(wrapper.text()).toContain('Loading older mail');
  expect(wrapper.get('[role="progressbar"]').attributes('aria-valuenow')).toBe('800');
  expect(wrapper.get('[role="progressbar"]').attributes('aria-valuemax')).toBe('1000');
  expect(wrapper.text()).not.toContain('800 of 1,000');
  expect(wrapper.get('button').attributes('title')).toContain('800 of 1,000 messages');
  wrapper.unmount();
});

it('never rounds an unfinished import up to 100%', () => {
  setActivePinia(createPinia());
  const connection = useConnectionStore();
  connection.syncProgress = [{ account_id: 'account', account_name: 'Work', email: 'me@example.test',
    phase: 'waiting', folder: null, folder_index: null, folder_total: null,
    backfill_total: 1000, backfill_remaining: 4, updated_at: '2026-11-12T12:00:00Z' }];
  const wrapper = mount(SyncStatusLine, { props: { hideIdle: true } });
  expect(wrapper.text()).toContain('99%');
  wrapper.unmount();
});

it('keeps history progress visible between runs and pauses honestly while offline', async () => {
  setActivePinia(createPinia());
  const connection = useConnectionStore();
  connection.syncProgress = [{ account_id: 'account', account_name: 'Work', email: 'me@example.test',
    phase: 'waiting', folder: null, folder_index: null, folder_total: null,
    backfill_total: 1000, backfill_remaining: 200, updated_at: '2026-11-12T12:00:00Z' }];
  const wrapper = mount(SyncStatusLine, { props: { hideIdle: true } });
  expect(wrapper.text()).toContain('Loading older mail');
  expect(wrapper.text()).toContain('80%');
  expect(wrapper.get('button').attributes('title')).toContain('Working in the background');
  connection.syncProgress = connection.syncProgress.map(row => ({ ...row, backfill_remaining: 100 }));
  await wrapper.vm.$nextTick();
  expect(wrapper.get('[role="progressbar"]').attributes('aria-valuenow')).toBe('900');
  expect(wrapper.text()).toContain('90%');
  connection.online = false;
  await wrapper.vm.$nextTick();
  expect(wrapper.text()).toContain('Paused while offline');
  connection.syncProgress = [];
  connection.online = true;
  await wrapper.vm.$nextTick();
  expect(wrapper.find('[role="status"]').exists()).toBe(false);
  wrapper.unmount();
});

it('does not invent a percentage before a total is known', () => {
  setActivePinia(createPinia());
  useConnectionStore().syncing = true;
  const wrapper = mount(SyncStatusLine, { props: { hideIdle: true } });
  expect(wrapper.text()).toContain('Syncing…');
  expect(wrapper.text()).not.toContain('%');
  expect(wrapper.find('[role="progressbar"]').exists()).toBe(false);
  wrapper.unmount();
});

it('keeps short background polls out of the desktop footer', async () => {
  vi.useFakeTimers();
  setActivePinia(createPinia());
  const connection = useConnectionStore();
  connection.syncing = true;
  const wrapper = mount(SyncStatusLine, { props: { hideIdle: true, progressOnly: true } });
  try {
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
    await vi.advanceTimersByTimeAsync(3000);
    expect(wrapper.text()).toContain('Syncing…');
    connection.syncing = false;
    await wrapper.vm.$nextTick();
    expect(wrapper.find('[role="status"]').exists()).toBe(false);
  } finally {
    wrapper.unmount();
    vi.useRealTimers();
  }
});

it('includes completed accounts in the total while another account is still importing', () => {
  setActivePinia(createPinia());
  const connection = useConnectionStore();
  const row = { account_id: 'work', account_name: 'Work', email: 'me@example.test', phase: 'waiting',
    folder: null, folder_index: null, folder_total: null, backfill_total: 1000, backfill_remaining: 200,
    updated_at: '2026-11-12T12:00:00Z' };
  connection.syncProgress = [row, { ...row, account_id: 'personal', phase: 'complete', backfill_total: 500, backfill_remaining: 0 }];
  const wrapper = mount(SyncStatusLine, { props: { hideIdle: true } });
  expect(wrapper.get('[role="progressbar"]').attributes('aria-valuenow')).toBe('1300');
  expect(wrapper.get('[role="progressbar"]').attributes('aria-valuemax')).toBe('1500');
  wrapper.unmount();
});
