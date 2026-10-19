import { onBeforeUnmount, watch, type Ref } from 'vue';
import {
  BACKUP_HEARTBEAT_MS,
  composerBackupPersistence,
  type ComposerBackup,
  type ComposerBackupContent,
} from '@/persistence/composerBackup';
import { hasMeaningfulComposerContent } from '@/utils/composerDraftState';

export interface ComposerLocalBackupOptions {
  enabled: Ref<boolean>;
  draftId: Ref<string | null>;
  content: () => ComposerBackupContent;
  debounceMs?: number;
}

/**
 * Keeps a device-local copy of what the composer holds until the draft is on
 * disk. Written ~300 ms after typing pauses; cleared once a save has caught up.
 */
export function useComposerLocalBackup(options: ComposerLocalBackupOptions) {
  const key = `composer-${crypto.randomUUID()}`;
  let pending: ReturnType<typeof setTimeout> | null = null;
  let heartbeat: ReturnType<typeof setInterval> | null = null;
  let stored = false;

  function stopHeartbeat() {
    if (heartbeat === null) return;
    clearInterval(heartbeat);
    heartbeat = null;
  }

  function writeNow() {
    pending = null;
    const content = options.content();
    if (!hasMeaningfulComposerContent(content)) {
      clear();
      return;
    }
    stored = composerBackupPersistence.write({
      key,
      draftId: options.draftId.value,
      savedAt: new Date().toISOString(),
      aliveAt: Date.now(),
      content,
    });
    if (stored && heartbeat === null) {
      heartbeat = setInterval(() => composerBackupPersistence.touch(key), BACKUP_HEARTBEAT_MS);
    }
  }

  function schedule() {
    if (pending !== null) clearTimeout(pending);
    pending = setTimeout(writeNow, options.debounceMs ?? 300);
  }

  function flush() {
    if (pending === null) return;
    clearTimeout(pending);
    writeNow();
  }

  /** The draft on disk has caught up with the editor. */
  function clear() {
    if (pending !== null) clearTimeout(pending);
    pending = null;
    stopHeartbeat();
    if (stored) composerBackupPersistence.remove(key);
    stored = false;
  }

  /** A copy a closed composer left behind, for this draft or, without one, any. */
  function findOrphan(draftId: string | null): ComposerBackup | null {
    return composerBackupPersistence.findOrphan({ draftId });
  }

  function discardOrphan(backup: ComposerBackup) {
    composerBackupPersistence.remove(backup.key);
  }

  watch(() => JSON.stringify(options.content()), () => {
    if (options.enabled.value) schedule();
  });

  const onPageHide = () => flush();
  if (typeof window !== 'undefined') window.addEventListener('pagehide', onPageHide);

  onBeforeUnmount(() => {
    if (typeof window !== 'undefined') window.removeEventListener('pagehide', onPageHide);
    flush();
    // A copy that outlives the composer becomes an orphan the next one offers.
    stopHeartbeat();
  });

  return { flush, clear, findOrphan, discardOrphan };
}
