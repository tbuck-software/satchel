import { onBeforeUnmount, ref, type Ref } from 'vue';
import { getCurrentWebviewWindow, type WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { isMobileRuntime } from '@/utils/runtimeShell';
import type { BackendDraftInput } from '@/types/composer';
import {
  composerDraftInput,
  composerSavedSnapshot,
  hasMeaningfulComposerContent,
  type ComposerDraftState,
} from '@/utils/composerDraftState';

interface ComposerCloseGuardOptions {
  embedded: () => boolean;
  standalone: () => boolean;
  supportsBackend: boolean;
  initialized: Ref<boolean>;
  currentState: () => ComposerDraftState;
  persistDraft: (options: { silent?: boolean }) => Promise<string | null>;
  finishDraftPersistence: () => void;
  closeDraftWithoutSaving: () => Promise<void>;
}

/** Owns change detection and native standalone-window close handling. */
export function useComposerCloseGuard(options: ComposerCloseGuardOptions) {
  /**
   * What closing would leave behind: the content at mount, then whatever the
   * last successful save put on disk. Comparing with the mount state alone let
   * an edit that was autosaved and then undone close without a save, leaving
   * the autosaved version on disk.
   */
  const baseline = ref('');
  const standaloneCloseBusy = ref(false);
  let standaloneWindow: WebviewWindow | null = null;
  let standaloneCloseUnlisten: (() => void) | null = null;
  let disposed = false;

  function currentSnapshot() {
    return composerSavedSnapshot(composerDraftInput(options.currentState()));
  }

  function captureBaseline() {
    baseline.value = currentSnapshot();
  }

  /** The draft on disk now matches `input`. */
  function markSaved(input: Omit<BackendDraftInput, 'id' | 'expectedRevision'>) {
    baseline.value = composerSavedSnapshot(input);
  }

  function hasMeaningfulContent() {
    return hasMeaningfulComposerContent(options.currentState());
  }

  function shouldConfirmClose() {
    if (!options.initialized.value || !hasMeaningfulContent()) return false;
    return currentSnapshot() !== baseline.value;
  }

  async function closeWithoutSaving() {
    await options.closeDraftWithoutSaving();
  }

  /**
   * A standalone window saves changed content before closing. It stays open
   * when that save fails because there is no other copy of the edit.
   */
  async function saveAndCloseStandalone() {
    if (!shouldConfirmClose()) {
      await closeWithoutSaving();
      await standaloneWindow?.destroy();
      return;
    }

    standaloneCloseBusy.value = true;
    const savedDraftId = await options.persistDraft({ silent: false });
    if (!savedDraftId) {
      // Saving failed while there is content. Keep the window open so the
      // sender can retry without losing their work.
      standaloneCloseBusy.value = false;
      return;
    }
    options.finishDraftPersistence();
    await standaloneWindow?.destroy();
  }

  /** A composer in a window of its own, as opposed to the phone's in-place route. */
  function isPopout() {
    return !options.embedded() && options.standalone() && !isMobileRuntime();
  }

  /** Closes the popout after a send; nothing is left to save there. */
  async function closePopout() {
    await options.closeDraftWithoutSaving();
    if (options.supportsBackend) {
      await (standaloneWindow ?? getCurrentWebviewWindow()).destroy();
    } else {
      window.close();
    }
  }

  async function installStandaloneCloseGuard() {
    if (options.embedded() || !options.standalone() || !options.supportsBackend) return;
    standaloneWindow = getCurrentWebviewWindow();
    const unlisten = await standaloneWindow.onCloseRequested((event) => {
      if (disposed) return;
      event.preventDefault();
      if (standaloneCloseBusy.value) return;
      void saveAndCloseStandalone();
    });
    if (disposed) {
      unlisten();
      return;
    }
    standaloneCloseUnlisten = unlisten;
  }

  onBeforeUnmount(() => {
    disposed = true;
    standaloneCloseUnlisten?.();
    standaloneCloseUnlisten = null;
  });

  return {
    captureBaseline,
    markSaved,
    hasMeaningfulContent,
    shouldConfirmClose,
    closeWithoutSaving,
    installStandaloneCloseGuard,
    isPopout,
    closePopout,
  };
}
