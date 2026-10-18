import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import {
  backendCommandsAvailable,
  backendCurrentConnectionState,
  backendSupportsNativeCommands,
  type BackendConnectionState,
} from '@/services/backend';
import { t } from '@/i18n/runtime';
import { backendErrorKey, backendErrorStringParam } from '@/utils/backendErrors';

/** Offline is a state, not a failure — only `error` belongs to the banner. */
export type SyncStatus = 'error' | 'offline' | 'syncing' | 'idle';

export const useConnectionStore = defineStore('connection', () => {
  // Browser/OS connectivity is a hint. Mail, credential and local DB errors do not change it.
  const online = ref(globalThis.navigator?.onLine ?? true);
  const syncing = ref(false);
  const lastSyncAt = ref<string | null>(null);
  const backfillRemaining = ref(0);
  const backfillTotal = ref(0);
  const pendingOutbox = ref(0);
  const pendingSends = ref(0);
  const failedOutbox = ref(0);
  const failedSends = ref(0);
  const failedRemoteMutations = ref(0);
  const isLoaded = ref(false);
  const backendErrors = ref<Record<string, string>>({});
  /** The encoded per-account failures behind a `sync` error, so the banner can act on each. */
  const syncFailures = ref<string[]>([]);
  const retryToken = ref(0);
  const retrying = ref(false);
  const backendError = computed(() => Object.values(backendErrors.value)[0] ?? null);
  /**
   * Which subsystem reported it, so the UI can name the right one.
   *
   * `sync` is the odd one out: the backend answered, and an account inside its
   * reply did not. Calling that "backend unreachable" pointed at the wrong end
   * of the wire and cost real time to chase.
   */
  const backendErrorSource = computed(() => Object.keys(backendErrors.value)[0] ?? null);

  const backfillRunning = computed(() => backfillRemaining.value > 0);
  const backfillDone = computed(() => Math.max(backfillTotal.value - backfillRemaining.value, 0));

  /**
   * One precedence chain for every surface that reports the connection, so the
   * sidebar line and `ConnectionBanner` cannot contradict each other. `error`
   * belongs to the banner: it is the same fact in an actionable form, and
   * saying it twice in two tones reads as two problems.
   */
  const syncStatus = computed<SyncStatus>(() => {
    if (backendError.value) return 'error';
    if (!online.value) return 'offline';
    if (syncing.value) return 'syncing';
    return 'idle';
  });

  /**
   * What one subsystem reported, so a view that came back empty can tell a
   * failed load from an empty mailbox. Raw on purpose: translating at report
   * time would freeze the language the app started in — the UI runs it through
   * `backendErrorLabel()` when it renders.
   */
  function backendErrorFor(source: string): string | null {
    return backendErrors.value[source] ?? null;
  }

  function reportBackendError(source: string, error: unknown) {
    if (!backendCommandsAvailable()) return;
    if (source === 'sync') syncFailures.value = [];
    const detail = error instanceof Error ? error.message : String(error);
    backendErrors.value = {
      ...backendErrors.value,
      [source]: detail || t('errors.backendUnknown'),
    };
  }

  /**
   * A sync run's per-account failures, kept untranslated: rendering them is
   * what follows the language, and each one names the account to fix.
   */
  function reportSyncFailures(failures: string[]) {
    if (failures.length === 0) {
      clearBackendError('sync');
      return;
    }
    reportBackendError('sync', failures.join('\n'));
    syncFailures.value = [...failures];
  }

  function clearBackendError(source: string) {
    if (source === 'sync') syncFailures.value = [];
    if (!(source in backendErrors.value)) return;
    const next = { ...backendErrors.value };
    delete next[source];
    backendErrors.value = next;
  }

  function clearAccountSyncFailure(accountId: string) {
    if (!syncFailures.value.some((failure) =>
      backendErrorKey(failure) === 'backend.errors.accountSyncFailed'
      && backendErrorStringParam(failure, 'accountId') === accountId)) return;
    reportSyncFailures(syncFailures.value.filter((failure) =>
      backendErrorKey(failure) !== 'backend.errors.accountSyncFailed'
      || backendErrorStringParam(failure, 'accountId') !== accountId));
  }

  function requestRetry() {
    retryToken.value += 1;
  }

  /**
   * The first sync of a new account backfills for hours. The counts only exist
   * in the `run_imap_sync` reply, so whoever runs a sync hands them over here —
   * otherwise the inbox says "no threads" while 40 000 messages are on the way.
   */
  function applySyncSummary(summary: {
    backfill_remaining_messages: number;
    backfill_total_messages: number;
  }) {
    backfillRemaining.value = Math.max(summary.backfill_remaining_messages, 0);
    backfillTotal.value = Math.max(summary.backfill_total_messages, 0);
  }

  function applyState(state: BackendConnectionState | Partial<BackendConnectionState>) {
    // Backend commands describe mail work, not the device network.
    if (typeof state.syncing === 'boolean') syncing.value = state.syncing;
    if ('last_sync_at' in state && state.last_sync_at !== undefined) {
      lastSyncAt.value = state.last_sync_at ?? null;
    }
    if (typeof state.pending_outbox === 'number') pendingOutbox.value = state.pending_outbox;
    if (typeof state.pending_sends === 'number') pendingSends.value = state.pending_sends;
    if (typeof state.failed_outbox === 'number') failedOutbox.value = state.failed_outbox;
    if (typeof state.failed_sends === 'number') failedSends.value = state.failed_sends;
    if (typeof state.failed_remote_mutations === 'number') {
      failedRemoteMutations.value = state.failed_remote_mutations;
    }
    isLoaded.value = true;
  }

  let syncRevision = 0;
  let refreshRequest = 0;

  async function refresh() {
    if (!backendCommandsAvailable()) return;
    const request = ++refreshRequest;
    const session = generation;
    try {
      const observedRevision = syncRevision;
      const snapshot = await backendCurrentConnectionState();
      if (request !== refreshRequest || session !== generation) return;
      applyState(observedRevision === syncRevision ? snapshot : { ...snapshot, syncing: syncing.value });
      clearBackendError('connection');
    } catch (err) {
      if (request !== refreshRequest || session !== generation) return;
      reportBackendError('connection', err);
      console.warn('connection.refresh failed', err);
    }
  }

  let unlisten: (() => void) | null = null;
  let networkOnline: (() => void) | null = null;
  let networkOffline: (() => void) | null = null;

  let generation = 0;
  let started = false;

  async function start() {
    if (started) return;
    started = true;
    const session = ++generation;
    if (typeof window !== 'undefined') {
      networkOnline = () => { online.value = true; };
      networkOffline = () => { online.value = false; };
      window.addEventListener('online', networkOnline);
      window.addEventListener('offline', networkOffline);
      online.value = navigator.onLine;
    }

    // Subscribe first so a sync beginning during the initial request is visible.
    if (backendSupportsNativeCommands()) {
      try {
        const mod = await import('@tauri-apps/api/event');
        if (session !== generation) return;
        const release = await mod.listen<Partial<BackendConnectionState>>('connection:state', (event) => {
          if (session !== generation) return;
          if (typeof event.payload.syncing === 'boolean') syncRevision += 1;
          applyState(event.payload);
        });
        if (session !== generation) {
          release();
          return;
        }
        unlisten = release;
      } catch (err) {
        console.warn('connection:state listener failed', err);
      }
    }
    if (session === generation) await refresh();
  }

  function stop() {
    generation += 1;
    started = false;
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
    if (typeof window !== 'undefined') {
      if (networkOnline) window.removeEventListener('online', networkOnline);
      if (networkOffline) window.removeEventListener('offline', networkOffline);
      networkOnline = null;
      networkOffline = null;
    }
  }

  return {
    online,
    backendErrorSource,
    syncing,
    syncStatus,
    lastSyncAt,
    backfillRemaining,
    backfillTotal,
    backfillRunning,
    backfillDone,
    pendingOutbox,
    pendingSends,
    failedOutbox,
    failedSends,
    failedRemoteMutations,
    isLoaded,
    backendError,
    syncFailures,
    retryToken,
    retrying,
    backendErrorFor,
    applySyncSummary,
    reportBackendError,
    reportSyncFailures,
    clearBackendError,
    clearAccountSyncFailure,
    requestRetry,
    refresh,
    start,
    stop,
  };
});
