// @vitest-environment jsdom
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { useMailMergeStore } from '@/stores/mailMerge';
import type { BackendMergeJob, BackendMergeJobDetail } from '@/types/mailMerge';
import MailMergeJobsPanel from './MailMergeJobsPanel.vue';

const sendCommand = vi.hoisted(() => vi.fn());
vi.mock('@/services/backend/transport', () => ({
  backendCommandsAvailable: () => true,
  sendCommand,
}));

function pendingDetail() {
  let resolve!: (detail: BackendMergeJobDetail) => void;
  const promise = new Promise<BackendMergeJobDetail>((complete) => { resolve = complete; });
  return { promise, resolve };
}

function job(id: string): BackendMergeJob {
  return {
    id,
    account_id: 'account',
    status: 'completed',
    pause_reason: null,
    subject: `Update ${id}`,
    source_label: 'recipients.csv',
    delay_seconds: 60,
    next_send_at: null,
    created_at: '2026-11-20T10:00:00Z',
    updated_at: '2026-11-20T10:00:00Z',
    finished_at: '2026-11-20T10:00:00Z',
    total: 1,
    pending: 0,
    queued: 0,
    sent: 1,
    failed: 0,
    unknown: 0,
    cancelled: 0,
  };
}

enableAutoUnmount(afterEach);
beforeEach(() => {
  sendCommand.mockReset();
});

describe('mail merge jobs panel', () => {
  it('shows an initial load failure and retries from the banner', async () => {
    sendCommand.mockRejectedValueOnce(new Error('Storage temporarily unavailable'))
      .mockResolvedValueOnce([job('one')]);
    const wrapper = mount(MailMergeJobsPanel, { global: { plugins: [createPinia()] } });
    await flushPromises();

    const banner = wrapper.get('[role="alert"]');
    expect(banner.text()).toContain('Could not load the mail merge jobs.');
    expect(banner.text()).toContain('Storage temporarily unavailable');
    expect(wrapper.find('li').exists()).toBe(false);

    const retry = banner.get('button');
    expect(retry.text()).toBe('Try again');
    await retry.trigger('click');
    await flushPromises();

    expect(sendCommand.mock.calls.map(([command]) => command))
      .toEqual(['list_mail_merge_jobs', 'list_mail_merge_jobs']);
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(wrapper.text()).toContain('Update one');
  });
});

describe('mail merge recipient details', () => {
  it('ignores an answer after the detail was closed', async () => {
    const response = pendingDetail();
    sendCommand.mockReturnValueOnce(response.promise);
    const store = useMailMergeStore(createPinia());

    const opening = store.openDetail('one');
    store.closeDetail();
    response.resolve({ job: job('one'), recipients: [] });
    await opening;

    expect(store.detail).toBeNull();
    expect(store.detailJobId).toBeNull();
  });

  it('keeps the newly selected detail when the previous job answers late', async () => {
    const response = pendingDetail();
    sendCommand.mockReturnValueOnce(response.promise)
      .mockResolvedValueOnce({ job: job('two'), recipients: [] });
    const store = useMailMergeStore(createPinia());

    const opening = store.openDetail('one');
    await store.openDetail('two');
    response.resolve({ job: job('one'), recipients: [] });
    await opening;

    expect(store.detail?.job.id).toBe('two');
  });

  it.each(['reload', 'delete'] as const)('invalidates a pending detail when the job disappears after %s', async (cause) => {
    const response = pendingDetail();
    sendCommand.mockReturnValueOnce(response.promise);
    if (cause === 'delete') {
      sendCommand.mockResolvedValueOnce(null).mockResolvedValueOnce([job('one')]);
    } else {
      sendCommand.mockResolvedValueOnce([]);
    }
    const store = useMailMergeStore(createPinia());

    const opening = store.openDetail('one');
    if (cause === 'delete') await store.control('one', 'delete');
    else await store.load();
    response.resolve({ job: job('one'), recipients: [] });
    await opening;

    expect(store.detail).toBeNull();
    expect(store.detailJobId).toBeNull();
  });
});
