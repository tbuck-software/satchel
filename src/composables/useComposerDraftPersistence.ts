import {
  onBeforeUnmount,
  ref,
  watch,
  type Ref,
  type WatchSource,
} from 'vue';
import type { BackendDraft, BackendDraftInput } from '@/types/composer';
import { backendErrorKey, translateBackendError } from '@/utils/backendErrors';
import { createCoalescingRunner } from '@/utils/coalescingRunner';
import { normalizeVisibleText } from '@/utils/composerNormalization';

export type ComposerStatusTone = 'info' | 'success' | 'warning' | 'error';

/**
 * What the composer footer says, and how loudly.
 *
 * The tone is carried rather than derived at the call site: a failed send and a
 * saved draft used to be the same grey line, which is how a message that never
 * left could look like progress.
 */
export interface ComposerStatus {
  tone: ComposerStatusTone;
  text: string;
  /** Raw backend text for the collapsed technical details, if it adds anything. */
  raw?: string | null;
}

export type ComposerDraftTranslator = (
  key: string,
  params?: Record<string, string | number>,
) => string;

export interface ComposerDraftPersistenceAdapter {
  save(input: BackendDraftInput): Promise<BackendDraft | null>;
  remove(id: string): Promise<void>;
}

export interface ComposerDraftPersistenceOptions {
  initialized: Ref<boolean>;
  sending: Ref<boolean>;
  status: Ref<ComposerStatus | null>;
  trackedSources: WatchSource<unknown>[];
  buildInput: () => Omit<BackendDraftInput, 'id' | 'expectedRevision'> | null;
  drafts: ComposerDraftPersistenceAdapter;
  t: ComposerDraftTranslator;
  autosaveIntervalMs?: number;
  /** A save landed and nothing changed while it ran: the disk has caught up. */
  onCaughtUp?: () => void;
}

/**
 * Why the draft on disk is behind the editor. `conflict` means another window
 * changed, sent or discarded it; saving again under the same id cannot help.
 */
export interface ComposerSaveError {
  kind: 'failed' | 'conflict';
  message: string;
}

const CONFLICT_KEYS = new Set([
  'backend.errors.draftChangedElsewhere',
  'backend.errors.draftGoneElsewhere',
]);

interface DraftSaveOutcome {
  id: string | null;
  remoteError: string | null;
  error: ComposerSaveError | null;
}

/**
 * Every composer alive in this window, so quitting can reach all of them.
 *
 * The autosave runs every ten seconds; a quit through the tray menu or ⌘Q used
 * to end the process without asking, so up to ten seconds of typing went with
 * it. `bootstrap.ts` calls this when the backend says the app is on its way
 * out, and answers once it resolves.
 */
const openComposerFlushes = new Set<() => Promise<boolean>>();

/** `false` when any composer could not put its draft on disk. */
export async function flushOpenComposerDrafts(): Promise<boolean> {
  const results = await Promise.allSettled(Array.from(openComposerFlushes, (flush) => flush()));
  return results.every((result) => result.status === 'fulfilled' && result.value);
}

/** Owns draft identity, save coalescing, autosave, and terminal cleanup. */
export function useComposerDraftPersistence(options: ComposerDraftPersistenceOptions) {
  const draftId = ref<string | null>(null);
  /** The revision the backend last confirmed for `draftId`. */
  const serverRevision = ref<number | null>(null);
  const dirty = ref(false);
  const saveError = ref<ComposerSaveError | null>(null);
  let revision = 0;
  let paused = false;
  let autosaveTimer: ReturnType<typeof setInterval> | null = null;

  const runner = createCoalescingRunner(saveOnce, () => dirty.value);

  watch(options.trackedSources, () => {
    if (!options.initialized.value) return;
    revision += 1;
    dirty.value = true;
  });

  function isEmptyNewDraft(input: Omit<BackendDraftInput, 'id' | 'expectedRevision'>) {
    return (
      !draftId.value
      && (input.to?.length ?? 0) === 0
      && !normalizeVisibleText(input.subject ?? '')
      && !normalizeVisibleText(input.body ?? '')
      && (input.attachments?.length ?? 0) === 0
    );
  }

  async function saveOnce(): Promise<DraftSaveOutcome> {
    const input = options.buildInput();
    if (!input || isEmptyNewDraft(input)) {
      return { id: null, remoteError: null, error: null };
    }

    // Publish the identity before the adapter gets a chance to yield. Calls
    // received during that save must address the same local draft.
    if (!draftId.value) draftId.value = `draft-${crypto.randomUUID()}`;
    const revisionAtStart = revision;

    try {
      const draft = await options.drafts.save({
        ...input,
        id: draftId.value,
        expectedRevision: serverRevision.value,
      });
      if (draft) {
        draftId.value = draft.id;
        serverRevision.value = draft.revision ?? null;
      }
      dirty.value = revision !== revisionAtStart;
      saveError.value = null;
      if (!dirty.value) options.onCaughtUp?.();
      // The local copy is durable even when the provider rejects its remote
      // copy. Report that state without turning a successful local save into
      // a failure that would keep the composer open.
      const remoteError = draft?.remote_error?.trim();
      return {
        id: draft?.id ?? null,
        remoteError: remoteError || null,
        error: null,
      };
    } catch (error) {
      console.warn('draft save failed', error);
      const key = backendErrorKey(error);
      const message = translateBackendError(error)
        ?? (error instanceof Error ? error.message : String(error));
      saveError.value = {
        kind: key && CONFLICT_KEYS.has(key) ? 'conflict' : 'failed',
        message,
      };
      return { id: null, remoteError: null, error: saveError.value };
    }
  }

  async function persist(saveOptions: { silent?: boolean } = {}): Promise<string | null> {
    if (paused) return null;
    const outcome = await runner.run();
    if (outcome.remoteError) {
      options.status.value = {
        tone: 'warning',
        text: options.t('compose.status.draftSavedRemotePending', {
          error: outcome.remoteError,
        }),
      };
    } else if (outcome.error) {
      // `saveError` carries the failure; a stale "Draft saved." must not sit next to it.
      if (options.status.value?.tone !== 'error') options.status.value = null;
    } else if (outcome.id && !saveOptions.silent) {
      options.status.value = { tone: 'info', text: options.t('compose.status.draftSaved') };
    }
    return outcome.id;
  }

  function stopAutosave() {
    if (autosaveTimer === null) return;
    clearInterval(autosaveTimer);
    autosaveTimer = null;
  }

  function startAutosave() {
    stopAutosave();
    autosaveTimer = setInterval(() => {
      // A conflict does not heal by retrying; the user picks a way out.
      if (saveError.value?.kind === 'conflict') return;
      if (dirty.value && !options.sending.value && !paused) {
        void persist({ silent: true });
      }
    }, options.autosaveIntervalMs ?? 10_000);
  }

  async function pause() {
    paused = true;
    runner.cancelPending();
    await runner.settle();
  }

  function resume() {
    paused = false;
  }

  function finish() {
    dirty.value = false;
    stopAutosave();
  }

  async function discard(discardOptions: { resumeAutosave: boolean }) {
    stopAutosave();
    await pause();
    try {
      dirty.value = false;
      if (draftId.value) {
        await options.drafts.remove(draftId.value);
        draftId.value = null;
        serverRevision.value = null;
      }
      saveError.value = null;
    } catch (error) {
      resume();
      startAutosave();
      throw error;
    }
    if (discardOptions.resumeAutosave) {
      resume();
      startAutosave();
    }
  }

  async function closeWithoutSaving() {
    await pause();
    dirty.value = false;
    stopAutosave();
  }

  /**
   * Leaves the conflicting draft alone and saves this editor's content as a
   * draft of its own.
   */
  async function saveAsNewDraft(): Promise<string | null> {
    draftId.value = null;
    serverRevision.value = null;
    saveError.value = null;
    dirty.value = true;
    revision += 1;
    return persist();
  }

  /** What a quit waits for. `false` keeps the app open: the edit is not on disk. */
  async function flushBeforeExit(): Promise<boolean> {
    if (options.sending.value || paused) return true;
    if (!dirty.value) return saveError.value === null;
    await persist({ silent: true });
    return saveError.value === null;
  }

  openComposerFlushes.add(flushBeforeExit);

  onBeforeUnmount(() => {
    openComposerFlushes.delete(flushBeforeExit);
    stopAutosave();
    if (dirty.value && !options.sending.value && !paused) {
      void persist({ silent: true });
    }
  });

  return {
    draftId,
    serverRevision,
    dirty,
    saveError,
    saveAsNewDraft,
    persist,
    startAutosave,
    pause,
    resume,
    finish,
    discard,
    closeWithoutSaving,
  };
}
