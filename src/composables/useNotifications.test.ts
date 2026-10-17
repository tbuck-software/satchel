import { beforeEach, describe, expect, it, vi } from 'vitest';

const notificationPlugin = vi.hoisted(() => ({
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
  sendNotification: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-notification', () => notificationPlugin);

import { ensureNotificationPermission, useNotifications } from './useNotifications';

describe('mobile notification permission', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });
  });

  it('requests the native permission when it has not been granted yet', async () => {
    notificationPlugin.isPermissionGranted.mockResolvedValue(false);
    notificationPlugin.requestPermission.mockResolvedValue('granted');

    await expect(ensureNotificationPermission()).resolves.toBe(true);
    expect(notificationPlugin.requestPermission).toHaveBeenCalledOnce();
  });

  it('does not send a notification after permission is denied', async () => {
    notificationPlugin.isPermissionGranted.mockResolvedValue(false);
    notificationPlugin.requestPermission.mockResolvedValue('denied');

    await expect(useNotifications().notify('Neue Mail', 'Nachricht')).resolves.toBe(false);
    expect(notificationPlugin.sendNotification).not.toHaveBeenCalled();
  });

  it('uses the native Tauri notification plugin after permission is granted', async () => {
    notificationPlugin.isPermissionGranted.mockResolvedValue(true);

    await expect(useNotifications().notify('Neue Mail', 'Nachricht')).resolves.toBe(true);
    expect(notificationPlugin.sendNotification).toHaveBeenCalledWith({
      title: 'Neue Mail',
      body: 'Nachricht',
    });
  });
});
