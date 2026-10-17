// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
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

    expect(mount(SyncStatusLine).get('[role="status"]').text()).toContain('Syncing …');
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
