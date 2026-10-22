import { invoke } from './core';
import type { AppUpdateCheck } from './generated/app/commands/app_update/AppUpdateCheck';
import type { AppUpdatePreferences } from './generated/app/commands/app_update/AppUpdatePreferences';
import type { UpdateChannel } from './generated/app/commands/app_update/UpdateChannel';
export type { UpdateChannel };

export function backendGetAppUpdatePreferences(): Promise<AppUpdatePreferences> {
  return invoke<AppUpdatePreferences>('get_app_update_preferences');
}

export function backendSetAppUpdateChannel(channel: UpdateChannel): Promise<void> {
  return invoke<void>('set_app_update_channel', { channel });
}

export function backendCheckAppUpdate(): Promise<AppUpdateCheck> {
  return invoke<AppUpdateCheck>('check_app_update');
}

export async function backendDownloadAppUpdate(reportProgress: (percent: number) => void): Promise<void> {
  let polling = false;
  const timer = setInterval(() => {
    if (polling) return;
    polling = true;
    void invoke<number>('app_update_progress')
      .then(reportProgress)
      .catch(() => { /* A progress read must not cancel the download. */ })
      .finally(() => { polling = false; });
  }, 500);
  try {
    await invoke<void>('download_app_update');
    reportProgress(100);
  } finally {
    clearInterval(timer);
  }
}

export function backendInstallAppUpdate(): Promise<void> {
  return invoke<void>('install_app_update');
}

export function backendAcknowledgeAppUpdateFlush(requestId: string, saved: boolean): Promise<void> {
  return invoke<void>('acknowledge_app_update_flush', { requestId, saved });
}
