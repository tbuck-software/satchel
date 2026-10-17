import { invoke } from './core';
import type { AppUpdateCheck } from './generated/app/commands/app_update/AppUpdateCheck';

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
