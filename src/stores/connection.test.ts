import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { backendCurrentConnectionState } from '@/services/backend';
import { useConnectionStore } from './connection';
import { listen } from '@tauri-apps/api/event';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }));

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendCurrentConnectionState: vi.fn(),
}));

type ConnectionSnapshot = Awaited<ReturnType<typeof backendCurrentConnectionState>>;

function deferredSnapshot() {
  let resolve!: (state: ConnectionSnapshot) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<ConnectionSnapshot>((done, failed) => {
    resolve = done;
    reject = failed;
  });
  return { promise, resolve, reject };
}

function snapshot(pending: number, syncing = false): ConnectionSnapshot {
  return {
    online: true, syncing, last_sync_at: null, pending_outbox: pending, pending_sends: pending,
    failed_outbox: 0, failed_sends: 0, failed_remote_mutations: 0,
  };
}

describe('connection backend errors', () => {
  beforeEach(() => {
    vi.stubGlobal('navigator', { onLine: true });
    vi.stubGlobal('window', new EventTarget());
    setActivePinia(createPinia());
    vi.mocked(backendCurrentConnectionState).mockReset();
    vi.mocked(listen).mockClear();
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
  });

  afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

  it('surfaces native backend failures and exposes a retry signal', async () => {
    vi.mocked(backendCurrentConnectionState).mockRejectedValue(new Error('backend stopped'));
    const store = useConnectionStore();
    await store.refresh();
    expect(store.backendError).toBe('backend stopped');
    expect(store.online).toBe(true);
    store.requestRetry();
    expect(store.retryToken).toBe(1);
  });

  it('clears a source error after recovery', () => {
    const store = useConnectionStore();
    store.reportBackendError('threads', new Error('failed'));
    expect(store.backendError).toBe('failed');
    expect(store.backendErrorSource).toBe('threads');
    store.clearBackendError('threads');
    expect(store.backendError).toBeNull();
    expect(store.backendErrorSource).toBeNull();
  });

  it('answers per subsystem, so an empty view can tell a failed load from no mail', () => {
    const store = useConnectionStore();
    store.reportBackendError('threads', new Error('db is locked'));

    expect(store.backendErrorFor('threads')).toBe('db is locked');
    expect(store.backendErrorFor('messages')).toBeNull();

    store.clearBackendError('threads');
    expect(store.backendErrorFor('threads')).toBeNull();
  });

  it('keeps backend errors independent of network availability', () => {
    const store = useConnectionStore();
    store.reportBackendError('threads', new Error('db is locked'));
    expect(store.online).toBe(true);
    expect(store.syncStatus).toBe('error');

    store.clearBackendError('threads');

    expect(store.online).toBe(true);
    expect(store.syncStatus).toBe('idle');
  });

  it('preserves an offline network hint when a local command recovers', () => {
    const store = useConnectionStore();
    store.online = false;
    store.reportBackendError('threads', new Error('db is locked'));
    store.clearBackendError('threads');
    expect(store.online).toBe(false);
    expect(store.syncStatus).toBe('offline');
  });

  it('does not mistake an account failure for a disconnected network', () => {
    const store = useConnectionStore();
    store.reportSyncFailures(['{"key":"backend.errors.accountAuthFailed","params":{"account":"work"}}']);
    expect(store.online).toBe(true);
    expect(store.syncStatus).toBe('error');
    expect(store.syncFailures).toEqual(['{"key":"backend.errors.accountAuthFailed","params":{"account":"work"}}']);
  });

  it('uses browser hints through successful local refreshes and cleans up listeners', async () => {
    vi.stubGlobal('navigator', { onLine: false });
    vi.mocked(backendCurrentConnectionState).mockResolvedValue({
      online: true, syncing: false, last_sync_at: null, pending_outbox: 0, pending_sends: 0,
      failed_outbox: 0, failed_sends: 0, failed_remote_mutations: 0,
    });
    const store = useConnectionStore();
    await store.start();
    expect(store.online).toBe(false);
    expect(store.syncStatus).toBe('offline');
    window.dispatchEvent(new Event('online'));
    expect(store.online).toBe(true);
    window.dispatchEvent(new Event('offline'));
    await store.refresh();
    expect(store.online).toBe(false);
    store.stop();
    window.dispatchEvent(new Event('online'));
    expect(store.online).toBe(false);
  });

  it('applies sync lifecycle events before the initial snapshot finishes', async () => {
    let resolveSnapshot: ((state: Awaited<ReturnType<typeof backendCurrentConnectionState>>) => void) | undefined;
    vi.mocked(backendCurrentConnectionState).mockReturnValue(new Promise(resolve => { resolveSnapshot = resolve; }));
    const store = useConnectionStore();
    const starting = store.start();
    await vi.waitFor(() => expect(listen).toHaveBeenCalled());
    const handler = vi.mocked(listen).mock.calls.at(-1)?.[1];
    expect(handler).toBeDefined();
    handler?.({ event: 'connection:state', id: 1, payload: { syncing: true } });
    expect(store.syncing).toBe(true);
    resolveSnapshot?.({
      online: true, syncing: false, last_sync_at: null, pending_outbox: 0, pending_sends: 0,
      failed_outbox: 0, failed_sends: 0, failed_remote_mutations: 0,
    });
    await starting;
    // The snapshot began before the lifecycle event; it cannot erase that event.
    expect(store.syncing).toBe(true);
    store.stop();
  });

  it('keeps the newest concurrent snapshot when requests resolve in reverse order', async () => {
    const older = deferredSnapshot();
    const newer = deferredSnapshot();
    vi.mocked(backendCurrentConnectionState)
      .mockReturnValueOnce(older.promise)
      .mockReturnValueOnce(newer.promise);
    const store = useConnectionStore();
    const first = store.refresh();
    const second = store.refresh();
    newer.resolve(snapshot(9, true));
    await second;
    older.resolve(snapshot(1));
    await first;
    expect(store.pendingOutbox).toBe(9);
    expect(store.syncing).toBe(true);
  });

  it('ignores an older failed refresh after a newer request recovered', async () => {
    const older = deferredSnapshot();
    vi.mocked(backendCurrentConnectionState)
      .mockReturnValueOnce(older.promise)
      .mockResolvedValueOnce(snapshot(9));
    const store = useConnectionStore();
    const first = store.refresh();
    await store.refresh();
    older.reject(new Error('stale failure'));
    await first;
    expect(store.backendError).toBeNull();
    expect(store.pendingOutbox).toBe(9);
  });

  it('does not apply a pending startup snapshot after stop', async () => {
    const pending = deferredSnapshot();
    vi.mocked(backendCurrentConnectionState).mockReturnValueOnce(pending.promise);
    const store = useConnectionStore();
    const starting = store.start();
    await vi.waitFor(() => expect(backendCurrentConnectionState).toHaveBeenCalledOnce());
    store.stop();
    pending.resolve(snapshot(9, true));
    await starting;
    expect(store.isLoaded).toBe(false);
    expect(store.pendingOutbox).toBe(0);
    expect(store.syncing).toBe(false);
  });

  it('keeps a restarted session when the previous startup finishes later', async () => {
    const older = deferredSnapshot();
    const newer = deferredSnapshot();
    vi.mocked(backendCurrentConnectionState)
      .mockReturnValueOnce(older.promise)
      .mockReturnValueOnce(newer.promise);
    const store = useConnectionStore();
    const first = store.start();
    await vi.waitFor(() => expect(backendCurrentConnectionState).toHaveBeenCalledTimes(1));
    store.stop();
    const second = store.start();
    await vi.waitFor(() => expect(backendCurrentConnectionState).toHaveBeenCalledTimes(2));
    newer.resolve(snapshot(9, true));
    await second;
    older.resolve(snapshot(1));
    await first;
    expect(store.pendingOutbox).toBe(9);
    expect(store.syncing).toBe(true);
    store.stop();
  });

  it('ranks the connection states so two surfaces cannot disagree', () => {
    const store = useConnectionStore();
    store.syncing = true;
    expect(store.syncStatus).toBe('syncing');

    store.online = false;
    expect(store.syncStatus).toBe('offline');

    store.reportBackendError('sync', new Error('host unreachable'));
    expect(store.syncStatus).toBe('error');
  });

  it('carries the backfill counts a sync reply brought back', () => {
    const store = useConnectionStore();
    expect(store.backfillRunning).toBe(false);

    store.applySyncSummary({
      backfill_remaining_messages: 400,
      backfill_total_messages: 1000,
    });

    expect(store.backfillRunning).toBe(true);
    expect(store.backfillDone).toBe(600);
  });

  it('keeps failed mail separate from failed internal sync actions', async () => {
    vi.mocked(backendCurrentConnectionState).mockResolvedValue({
      online: true,
      syncing: false,
      last_sync_at: null,
      pending_outbox: 12,
      pending_sends: 2,
      failed_outbox: 5,
      failed_sends: 1,
      failed_remote_mutations: 4,
    });
    const store = useConnectionStore();
    await store.refresh();
    expect(store.failedSends).toBe(1);
    expect(store.failedRemoteMutations).toBe(4);
  });
});
