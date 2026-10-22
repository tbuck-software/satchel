// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useConnectionStore } from '@/stores/connection';
import { backendRunImapSync } from '@/services/backend';
import { useMailSync } from '@/composables/useMailSync';
import { useToast } from '@/composables/useToast';
import SyncButton from './SyncButton.vue';

vi.mock('@/services/backend', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend')>(),
  backendRunImapSync: vi.fn(),
}));

const summary = {
  accounts_synced: 1, accounts_failed: 0, folders_synced: 0, messages_synced: 0,
  backfill_pending_accounts: 1, backfill_remaining_messages: 5, backfill_total_messages: 10,
  failures: [],
};

beforeEach(() => {
  setActivePinia(createPinia());
  vi.mocked(backendRunImapSync).mockReset().mockResolvedValue(summary);
});

afterEach(() => useToast().clear());

describe('SyncButton', () => {
  it('keeps the same button while automatic sync and offline state change', async () => {
    const connection = useConnectionStore();
    const wrapper = mount(SyncButton);
    const button = wrapper.get('button').element;
    expect(wrapper.get('button').attributes('title')).toContain('Not synced yet');

    connection.syncing = true;
    await wrapper.vm.$nextTick();
    expect(wrapper.get('button').element).toBe(button);
    expect(wrapper.get('button').attributes('aria-busy')).toBe('true');
    expect(wrapper.get('svg').classes()).toContain('animate-spin');
    await wrapper.get('button').trigger('click');
    expect(backendRunImapSync).not.toHaveBeenCalled();

    connection.syncing = false;
    connection.online = false;
    await wrapper.vm.$nextTick();
    expect(wrapper.get('button').element).toBe(button);
    expect(wrapper.get('button').attributes('title')).toContain('Offline');
    expect(wrapper.get('button').attributes('disabled')).toBeDefined();
    expect(wrapper.get('svg').classes()).not.toContain('animate-spin');
  });

  it('shares a pending click with the palette and applies the backfill summary', async () => {
    let finish!: (result: typeof summary) => void;
    vi.mocked(backendRunImapSync).mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const wrapper = mount(SyncButton);
    await wrapper.get('button').trigger('click');
    expect(wrapper.get('button').attributes('aria-busy')).toBe('true');
    expect(wrapper.get('button').attributes('disabled')).toBeDefined();
    await useMailSync().syncMail();
    expect(backendRunImapSync).toHaveBeenCalledTimes(1);

    finish(summary);
    await flushPromises();
    expect(useConnectionStore().backfillRemaining).toBe(5);
    expect(wrapper.get('button').attributes('aria-busy')).toBe('false');
    expect(wrapper.get('button').attributes('disabled')).toBeUndefined();
    expect(useToast().state.toasts.at(-1)?.message).toBe('Sync finished.');
  });

  it('releases the button after a failed sync and allows retrying', async () => {
    vi.mocked(backendRunImapSync).mockRejectedValueOnce(new Error('unavailable'));
    const wrapper = mount(SyncButton);
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(useToast().state.toasts.at(-1)?.tone).toBe('error');
    expect(wrapper.get('button').attributes('disabled')).toBeUndefined();
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(backendRunImapSync).toHaveBeenCalledTimes(2);
  });

  it('does not start window dragging on mousedown', async () => {
    const wrapper = mount(SyncButton);
    const parentMousedown = vi.fn();
    const parent = document.createElement('div');
    parent.addEventListener('mousedown', parentMousedown);
    parent.append(wrapper.element);
    await wrapper.get('button').trigger('mousedown');
    expect(parentMousedown).not.toHaveBeenCalled();
  });
});
