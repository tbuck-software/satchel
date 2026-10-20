import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { listen } from '@tauri-apps/api/event';
import {
  backendCancelOutboxAction,
  backendListOutbox,
  backendListFailedSyncActions,
  backendDiscardFailedSyncAction,
  backendRetryOutboxAction,
  type BackendOutboxItem,
} from '@/services/backend';
import { useConnectionStore } from './connection';
import { OUTBOX_EVENTS, useOutboxStore } from './outbox';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendListOutbox: vi.fn(),
  backendListFailedSyncActions: vi.fn(async () => []),
  backendDiscardFailedSyncAction: vi.fn(),
  backendRetryOutboxAction: vi.fn(),
  backendCancelOutboxAction: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

const listMock = vi.mocked(backendListOutbox);
const retryMock = vi.mocked(backendRetryOutboxAction);
const cancelMock = vi.mocked(backendCancelOutboxAction);
const listenMock = vi.mocked(listen);

function outboxItem(overrides: Partial<BackendOutboxItem> = {}): BackendOutboxItem {
  return {
    id: 'q-1',
    account_id: 'acc-1',
    action_type: 'send_message',
    status: 'failed',
    retry_count: 1,
    created_at: '2026-08-01T10:00:00Z',
    updated_at: '2026-08-01T10:01:00Z',
    recipients: ['mara@example.com'],
    subject: 'An Mara',
    last_error: 'smtp unavailable',
    next_retry_at: null,
    scheduled_for: null,
    ...overrides,
  };
}

describe('outbox store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('lists failed moves and discards only the selected move', async () => {
    listMock.mockResolvedValue([outboxItem()]);
    vi.mocked(backendListFailedSyncActions).mockResolvedValueOnce([outboxItem({ id: 'failed-move', action_type: 'remote_mutation' })]);
    const outbox = useOutboxStore();
    await outbox.load();
    vi.spyOn(useConnectionStore(), 'refresh').mockResolvedValue(undefined);
    expect(outbox.items.map((item) => item.id)).toEqual(['failed-move', 'q-1']);
    await outbox.cancel('failed-move');
    expect(backendDiscardFailedSyncAction).toHaveBeenCalledWith('failed-move');
    expect(backendCancelOutboxAction).not.toHaveBeenCalled();
    expect(outbox.items.map((item) => item.id)).toEqual(['q-1']);
  });

  it('loads queued actions and resets the loading flag afterwards', async () => {
    listMock.mockResolvedValue([outboxItem()]);
    const outbox = useOutboxStore();

    await outbox.load();

    expect(outbox.items).toHaveLength(1);
    expect(outbox.loading).toBe(false);
  });

  it('keeps the last items and records the cause when listing fails', async () => {
    listMock.mockResolvedValueOnce([outboxItem()]);
    const outbox = useOutboxStore();
    await outbox.load();

    listMock.mockRejectedValueOnce(new Error('backend gone'));
    await expect(outbox.load()).resolves.toBeUndefined();

    expect(outbox.error).toBe('backend gone');
    expect(outbox.items).toHaveLength(1);
    expect(outbox.loading).toBe(false);
    expect(useConnectionStore().backendErrorFor('outbox')).toBe('backend gone');

    listMock.mockResolvedValueOnce([]);
    await outbox.load();

    expect(outbox.error).toBeNull();
    expect(useConnectionStore().backendErrorFor('outbox')).toBeNull();
  });

  it('registers the live listeners even when the first load fails', async () => {
    listMock.mockRejectedValue(new Error('db is locked'));
    const outbox = useOutboxStore();

    await expect(outbox.start()).resolves.toBeUndefined();

    expect(listenMock.mock.calls.map(([eventName]) => eventName)).toEqual([
      'outbox:failed',
      'outbox:queued',
      'outbox:retry',
      'outbox:scheduled',
      'outbox:sent',
    ]);
    expect(outbox.error).toBe('db is locked');
  });

  it('publishes only the newest of overlapping loads', async () => {
    let resolveFirst: (items: BackendOutboxItem[]) => void = () => {};
    listMock
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
      .mockResolvedValueOnce([outboxItem({ id: 'fresh' })]);
    const outbox = useOutboxStore();

    const stale = outbox.load();
    await outbox.load();
    resolveFirst([outboxItem({ id: 'stale' })]);
    await stale;

    expect(outbox.items.map((item) => item.id)).toEqual(['fresh']);
    expect(outbox.loading).toBe(false);
  });

  it('registers listeners once and releases them when stopped mid-start', async () => {
    listMock.mockResolvedValue([]);
    const released = vi.fn();
    let finishListen: () => void = () => {};
    const gate = new Promise<void>((resolve) => { finishListen = resolve; });
    listenMock.mockImplementation(async () => {
      await gate;
      return released;
    });
    const outbox = useOutboxStore();

    const starting = outbox.start();
    const duplicate = outbox.start();
    await vi.waitFor(() => expect(listenMock).toHaveBeenCalledTimes(OUTBOX_EVENTS.length));
    outbox.stop();
    finishListen();
    await Promise.all([starting, duplicate]);

    expect(released).toHaveBeenCalledTimes(OUTBOX_EVENTS.length);
    expect(listMock).not.toHaveBeenCalled();
    listenMock.mockImplementation(async () => () => {});
  });

  it('removes an item only after the backend accepted the retry', async () => {
    listMock.mockResolvedValue([outboxItem(), outboxItem({ id: 'q-2' })]);
    const outbox = useOutboxStore();
    await outbox.load();

    retryMock.mockResolvedValueOnce(undefined);
    await outbox.retry('q-1');
    expect(outbox.items.map((item) => item.id)).toEqual(['q-2']);

    // A failed retry must not remove the item locally — the send is still pending.
    retryMock.mockRejectedValueOnce(new Error('still failing'));
    await expect(outbox.retry('q-2')).rejects.toThrow('still failing');
    expect(outbox.items.map((item) => item.id)).toEqual(['q-2']);
  });

  it('removes an item only after the backend accepted the cancellation', async () => {
    listMock.mockResolvedValue([outboxItem()]);
    const outbox = useOutboxStore();
    await outbox.load();

    cancelMock.mockRejectedValueOnce(new Error('already sent'));
    await expect(outbox.cancel('q-1')).rejects.toThrow('already sent');
    expect(outbox.items).toHaveLength(1);

    cancelMock.mockResolvedValueOnce(undefined);
    await outbox.cancel('q-1');
    expect(outbox.items).toHaveLength(0);
  });
});
