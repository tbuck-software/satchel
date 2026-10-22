import { computed, ref } from 'vue';
import { defineStore } from 'pinia';
import {
  backendCheckAppUpdate,
  backendCommandsAvailable,
  backendDownloadAppUpdate,
  backendInstallAppUpdate,
  backendGetAppUpdatePreferences,
  backendSetAppUpdateChannel,
  type UpdateChannel,
} from '@/services/backend';
import { useToast } from '@/composables/useToast';
import { t } from '@/i18n/runtime';

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

const UPDATE_TOAST = 'app-update';

/** The installed app follows one persisted channel and downloads signed updates. */
export const useAppUpdateStore = defineStore('appUpdate', () => {
  const checking = ref(false);
  const checked = ref(false);
  const channel = ref<UpdateChannel>('stable');
  const supported = ref(false);
  const preferencesLoaded = ref(false);
  const changingChannel = ref(false);
  let preferencesRequest: Promise<void> | null = null;
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

  async function loadPreferences() {
    if (!backendCommandsAvailable()) return;
    if (preferencesRequest) return preferencesRequest;
    preferencesRequest = (async () => {
      try {
        const result = await backendGetAppUpdatePreferences();
        channel.value = result.channel;
        supported.value = result.supported;
        preferencesLoaded.value = true;
        checkError.value = null;
      } catch (reason) {
        checkError.value = reason instanceof Error ? reason.message : String(reason);
      }
    })().finally(() => { preferencesRequest = null; });
    return preferencesRequest;
  }

  async function setChannel(next: UpdateChannel): Promise<boolean> {
    if (!supported.value || changingChannel.value || checking.value || status.value === 'downloading' || installing) return false;
    if (next === channel.value) return true;
    changingChannel.value = true;
    try {
      await backendSetAppUpdateChannel(next);
      channel.value = next;
      clearUpdate();
      checked.value = false;
      checkError.value = null;
      void checkForUpdates();
      return true;
    } catch (reason) {
      checkError.value = reason instanceof Error ? reason.message : String(reason);
      return false;
    } finally {
      changingChannel.value = false;
    }
  }

  async function checkForUpdates() {
    if (!backendCommandsAvailable() || checking.value || status.value !== 'hidden') return;
    checking.value = true;
    checkError.value = null;
    try {
      if (!preferencesLoaded.value) await loadPreferences();
      if (!preferencesLoaded.value || !supported.value) return;
      const result = await backendCheckAppUpdate();
      checked.value = true;
      if (result.version) {
        offerUpdate({
          version: result.version,
          download: backendDownloadAppUpdate,
          restart: backendInstallAppUpdate,
        });
        await downloadUpdate();
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
    void loadPreferences();
    scheduleCheck(8_000);
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
    useToast().dismissGroup(UPDATE_TOAST);
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
        useToast().dismissGroup(UPDATE_TOAST);
        useToast().info(t('app.update.ready', { version: version.value ?? '' }), {
          timeout: 0,
          groupKey: UPDATE_TOAST,
          action: { label: t('app.update.restartNow'), handler: restartToUpdate },
        });
      }
    } catch (reason) {
      if (offer === activeOffer) {
        error.value = reason instanceof Error ? reason.message : String(reason);
        failedAction.value = 'download';
        status.value = 'failed';
        showFailure(downloadUpdate);
      }
    }
  }

  function showFailure(retry: () => Promise<unknown>) {
    useToast().dismissGroup(UPDATE_TOAST);
    useToast().error(error.value ?? t('app.update.failedRetry'), {
      groupKey: UPDATE_TOAST,
      timeout: 0,
      action: { label: t('common.retry'), handler: retry },
    });
  }

  async function restartToUpdate(): Promise<boolean> {
    if (installing) return false;
    if (
      !offer ||
      (status.value !== 'ready' && !(status.value === 'failed' && failedAction.value === 'restart'))
    ) return false;

    installing = true;
    try {
      error.value = null;
      await offer.restart();
      return true;
    } catch (reason) {
      error.value = reason instanceof Error ? reason.message : String(reason);
      failedAction.value = 'restart';
      status.value = 'failed';
      showFailure(restartToUpdate);
      return false;
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
    supported,
    preferencesLoaded,
    changingChannel,
    loadPreferences,
    setChannel,
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
