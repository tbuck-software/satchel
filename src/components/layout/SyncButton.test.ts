// @vitest-environment jsdom
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useConnectionStore } from '@/stores/connection';
import { backendRunImapSync } from '@/services/backend';
import { useMailSync } from '@/composables/useMailSync';
import { useToast } from '@/composables/useToast';
import SyncButton from './SyncButton.vue';

vi.mock('@/services/backend', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend')>(),
  backendCommandsAvailable: () => true,
  backendRunImapSync: vi.fn(),
}));

vi.mock('./SyncFailures.vue', () => ({ default: { name: 'SyncFailures', props: ['open'], template: '<span />' } }));
vi.mock('./SyncProgressDetails.vue', () => ({ default: { name: 'SyncProgressDetails', props: ['open'], template: '<span />' } }));

enableAutoUnmount(afterEach);

function mountSyncButton() {
  return mount(SyncButton, { global: { stubs: { SyncFailures: true, SyncProgressDetails: true } } });
}

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
    const wrapper = mountSyncButton();
    const button = wrapper.get('button').element;
    expect(wrapper.get('button').attributes('title')).toContain('Not synced yet');

    connection.syncing = true;
    await wrapper.vm.$nextTick();
    expect(wrapper.get('button').element).toBe(button);
    expect(wrapper.get('button').attributes('aria-busy')).toBe('true');
    expect(wrapper.get('svg').classes()).not.toContain('sync-button__icon--spinning');
    await wrapper.get('button').trigger('click');
    expect(backendRunImapSync).not.toHaveBeenCalled();

    connection.syncing = false;
    connection.online = false;
    await wrapper.vm.$nextTick();
    expect(wrapper.get('button').element).toBe(button);
    expect(wrapper.get('button').attributes('title')).toContain('Offline');
    expect(wrapper.get('button').attributes('disabled')).toBeDefined();
    expect(wrapper.get('svg').classes()).not.toContain('sync-button__icon--spinning');
  });

  it('shares a pending click with the palette and applies the backfill summary', async () => {
    let finish!: (result: typeof summary) => void;
    vi.mocked(backendRunImapSync).mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const wrapper = mountSyncButton();
    await wrapper.get('button').trigger('click');
    expect(wrapper.get('button').attributes('aria-busy')).toBe('true');
    expect(wrapper.get('button').attributes('disabled')).toBeUndefined();
    expect(wrapper.get('button').attributes('aria-haspopup')).toBe('dialog');
    await useMailSync().syncMail();
    expect(backendRunImapSync).toHaveBeenCalledTimes(1);

    finish(summary);
    await flushPromises();
    expect(useConnectionStore().backfillRemaining).toBe(5);
    expect(wrapper.get('button').attributes('aria-busy')).toBe('false');
    expect(wrapper.get('button').attributes('disabled')).toBeUndefined();
    expect(useToast().state.toasts.at(-1)?.message).toBe('Sync finished.');
  });

  it('clears a failure the last automatic run left standing once a manual sync runs clean', async () => {
    const connection = useConnectionStore();
    connection.reportSyncFailures(['{"key":"backend.errors.folderSyncFailed","params":{"account":"me@example.com","folder":"SPAM","message":"gone"}}']);
    expect(connection.backendErrorFor('sync')).not.toBeNull();

    await useMailSync().syncMail();

    expect(connection.backendErrorFor('sync')).toBeNull();
    expect(connection.syncFailures).toEqual([]);
  });

  it('shows a manual run\'s failures in the banner instead of a "done" toast', async () => {
    const failure = '{"key":"backend.errors.folderSyncFailed","params":{"account":"me@example.com","folder":"SPAM","message":"gone"}}';
    vi.mocked(backendRunImapSync).mockResolvedValueOnce({ ...summary, accounts_failed: 1, failures: [failure] });

    await useMailSync().syncMail();

    expect(useConnectionStore().syncFailures).toEqual([failure]);
    expect(useToast().state.toasts.some((toast) => toast.message === 'Sync finished.')).toBe(false);
  });

  it('releases the button after a failed sync and allows retrying', async () => {
    vi.mocked(backendRunImapSync).mockRejectedValueOnce(new Error('unavailable'));
    const wrapper = mountSyncButton();
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(useToast().state.toasts.at(-1)?.tone).toBe('error');
    expect(wrapper.get('button').attributes('disabled')).toBeUndefined();
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(backendRunImapSync).toHaveBeenCalledTimes(2);
  });

  it('spins for a run that lasts, but not for a moment of queue work or a history import', async () => {
    vi.useFakeTimers();
    const connection = useConnectionStore();
    const wrapper = mountSyncButton();
    const spinning = () => wrapper.get('svg').classes().includes('sync-button__icon--spinning');
    try {
      connection.syncing = true;
      await vi.advanceTimersByTimeAsync(500);
      expect(spinning()).toBe(false);
      connection.syncing = false;
      await vi.advanceTimersByTimeAsync(2000);
      expect(spinning()).toBe(false);

      connection.syncing = true;
      await vi.advanceTimersByTimeAsync(1000);
      expect(spinning()).toBe(true);
      connection.syncing = false;
      await wrapper.vm.$nextTick();
      // The turn in progress finishes first.
      expect(spinning()).toBe(true);
      await wrapper.get('svg').trigger('animationiteration');
      expect(spinning()).toBe(false);

      connection.syncProgress = [{ account_id: 'work', account_name: 'Work', email: 'me@example.test', phase: 'folder',
        folder: 'Archive', folder_index: 1, folder_total: 3, backfill_total: 1000, backfill_remaining: 200,
        updated_at: '2026-11-12T12:00:00Z' }];
      connection.syncing = true;
      await vi.advanceTimersByTimeAsync(5000);
      expect(spinning()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('spins at once on a click and stops without animation events', async () => {
    vi.useFakeTimers();
    let finish!: (result: typeof summary) => void;
    vi.mocked(backendRunImapSync).mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const wrapper = mountSyncButton();
    try {
      await wrapper.get('button').trigger('click');
      expect(wrapper.get('svg').classes()).toContain('sync-button__icon--spinning');
      finish(summary);
      await flushPromises();
      // No animationiteration under reduced motion (one 1 ms turn); the fallback ends it.
      await vi.advanceTimersByTimeAsync(1100);
      expect(wrapper.get('svg').classes()).not.toContain('sync-button__icon--spinning');
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not start window dragging on mousedown', async () => {
    const wrapper = mountSyncButton();
    const parentMousedown = vi.fn();
    const parent = document.createElement('div');
    parent.addEventListener('mousedown', parentMousedown);
    parent.append(wrapper.element);
    await wrapper.get('button').trigger('mousedown');
    expect(parentMousedown).not.toHaveBeenCalled();
  });
});

it('keeps failures available after toasts expire, including while offline or syncing', async () => {
  const connection = useConnectionStore();
  connection.failedSends = 1;
  connection.failedRemoteMutations = 1;
  connection.online = false;
  connection.syncing = true;
  const wrapper = mount(SyncButton, { global: { stubs: { SyncFailures: true } } });
  expect(wrapper.get('button').attributes('disabled')).toBeUndefined();
  expect(wrapper.get('button').attributes('aria-label')).toBe('2 items need attention');
  expect(wrapper.find('.sync-button__failure').exists()).toBe(true);
  await wrapper.get('button').trigger('click');
  expect(wrapper.get('button').attributes('aria-expanded')).toBe('true');
  expect(backendRunImapSync).not.toHaveBeenCalled();
  wrapper.unmount();
});

it('shows a failure before the next connection count snapshot arrives', async () => {
  const { useOutboxStore } = await import('@/stores/outbox');
  useOutboxStore().items = [{ id: 'new-failure', account_id: 'a', action_type: 'send_message',
    status: 'failed', retry_count: 1, created_at: '', updated_at: '', recipients: [] }];
  const wrapper = mount(SyncButton, { global: { stubs: { SyncFailures: true } } });
  expect(wrapper.get('button').attributes('aria-label')).toBe('1 item needs attention');
  expect(wrapper.find('.sync-button__failure').exists()).toBe(true);
  wrapper.unmount();
});
