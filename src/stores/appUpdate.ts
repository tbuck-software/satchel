import { computed, ref } from 'vue';
import { defineStore } from 'pinia';
import {
  backendCheckAppUpdate,
  backendCommandsAvailable,
  backendDownloadAppUpdate,
  backendInstallAppUpdate,
} from '@/services/backend';

export type AppUpdateStatus = 'hidden' | 'available' | 'downloading' | 'ready' | 'failed';

export interface AppUpdateOffer {
  version?: string;
  download: (reportProgress: (percent: number) => void) => Promise<void>;
  restart: () => Promise<void>;
}

function normalizedProgress(percent: number): number {
  if (!Number.isFinite(percent)) return 0;
  return Math.min(100, Math.max(0, Math.round(percent)));
}

/** Release builds supply signed offers; source builds and mobile have no feed. */
export const useAppUpdateStore = defineStore('appUpdate', () => {
  const checking = ref(false);
  const checked = ref(false);
  const channel = ref<string | null>(null);
  const checkError = ref<string | null>(null);
  let checkTimer: ReturnType<typeof setTimeout> | null = null;
  let running = false;
  const status = ref<AppUpdateStatus>('hidden');
  const progress = ref(0);
  const version = ref<string | null>(null);
  const error = ref<string | null>(null);
  const failedAction = ref<'download' | 'restart'>('download');
  let offer: AppUpdateOffer | null = null;
  let installing = false;

  const visible = computed(() => status.value !== 'hidden');

  async function checkForUpdates() {
    if (!backendCommandsAvailable() || checking.value || status.value !== 'hidden') return;
    checking.value = true;
    checkError.value = null;
    try {
      const result = await backendCheckAppUpdate();
      channel.value = result.channel;
      checked.value = true;
      if (result.version) {
        offerUpdate({
          version: result.version,
          download: backendDownloadAppUpdate,
          restart: backendInstallAppUpdate,
        });
      }
    } catch (reason) {
      checkError.value = reason instanceof Error ? reason.message : String(reason);
    } finally {
      checking.value = false;
    }
  }

  function scheduleCheck(delay: number) {
    checkTimer = setTimeout(() => {
      checkTimer = null;
      void checkForUpdates().finally(() => {
        if (running) scheduleCheck(6 * 60 * 60_000);
      });
    }, delay);
  }

  function start() {
    if (running || !backendCommandsAvailable()) return;
    running = true;
    scheduleCheck(2 * 60_000);
  }

  function stop() {
    running = false;
    if (checkTimer !== null) clearTimeout(checkTimer);
    checkTimer = null;
  }

  function offerUpdate(nextOffer: AppUpdateOffer) {
    offer = nextOffer;
    version.value = nextOffer.version ?? null;
    progress.value = 0;
    error.value = null;
    failedAction.value = 'download';
    status.value = 'available';
  }

  function clearUpdate() {
    offer = null;
    version.value = null;
    progress.value = 0;
    error.value = null;
    failedAction.value = 'download';
    status.value = 'hidden';
  }

  async function downloadUpdate() {
    if (!offer || (status.value !== 'available' && status.value !== 'failed')) return;

    const activeOffer = offer;
    status.value = 'downloading';
    progress.value = 0;
    error.value = null;

    try {
      await activeOffer.download((percent) => {
        if (offer !== activeOffer || status.value !== 'downloading') return;
        progress.value = Math.max(progress.value, normalizedProgress(percent));
      });
      if (offer === activeOffer) {
        progress.value = 100;
        status.value = 'ready';
      }
    } catch (reason) {
      if (offer === activeOffer) {
        error.value = reason instanceof Error ? reason.message : String(reason);
        failedAction.value = 'download';
        status.value = 'failed';
      }
    }
  }

  async function restartToUpdate() {
    if (installing) return;
    if (
      !offer ||
      (status.value !== 'ready' && !(status.value === 'failed' && failedAction.value === 'restart'))
    ) return;

    installing = true;
    try {
      error.value = null;
      await offer.restart();
    } catch (reason) {
      error.value = reason instanceof Error ? reason.message : String(reason);
      failedAction.value = 'restart';
      status.value = 'failed';
    } finally {
      installing = false;
    }
  }

  async function activate() {
    if (status.value === 'ready' || (status.value === 'failed' && failedAction.value === 'restart')) {
      await restartToUpdate();
      return;
    }
    await downloadUpdate();
  }

  return {
    checking,
    checked,
    channel,
    checkError,
    checkForUpdates,
    start,
    stop,
    status,
    progress,
    version,
    error,
    visible,
    offerUpdate,
    clearUpdate,
    downloadUpdate,
    restartToUpdate,
    activate,
  };
});
