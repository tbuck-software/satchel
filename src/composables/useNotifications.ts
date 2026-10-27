function runsInsideTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/**
 * Resolve to whether we may post a notification, asking once if necessary.
 *
 * Mobile has no ambient `Notification` API — the permission lives with the
 * Tauri plugin, and on Android 13+ the OS refuses to post anything until the
 * user has been asked. The browser path stays for `pnpm dev` in a plain tab.
 */
export async function ensureNotificationPermission(): Promise<boolean> {
  if (runsInsideTauri()) {
    const { isPermissionGranted, requestPermission } = await import('@tauri-apps/plugin-notification');
    if (await isPermissionGranted()) return true;
    return (await requestPermission()) === 'granted';
  }

  if (typeof Notification === 'undefined') return false;
  if (Notification.permission === 'granted') return true;
  if (Notification.permission === 'denied') return false;
  return (await Notification.requestPermission()) === 'granted';
}

export function useNotifications() {
  /** Returns whether the notification was actually posted. */
  async function notify(title: string, body: string): Promise<boolean> {
    if (!(await ensureNotificationPermission())) return false;

    if (runsInsideTauri()) {
      const { sendNotification } = await import('@tauri-apps/plugin-notification');
      sendNotification({ title, body });
    } else {
      new Notification(title, { body });
    }
    return true;
  }

  return {
    notify,
  };
}
