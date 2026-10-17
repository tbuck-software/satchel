import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendDismissMobileNotifications,
  backendRuntimeCapabilities,
} from './backend';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

describe('runtime capabilities backend contract', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });
  });

  it('dismisses mobile notifications through the native command', async () => {
    invokeMock.mockResolvedValue(undefined);
    await backendDismissMobileNotifications(['thread-1']);
    expect(invokeMock).toHaveBeenCalledWith('dismiss_mobile_notifications', { threadIds: ['thread-1'] });
  });

  it('loads the runtime capabilities from the native shell', async () => {
    invokeMock.mockResolvedValue({ platform: 'android', mobile: true });
    await expect(backendRuntimeCapabilities()).resolves.toMatchObject({ platform: 'android', mobile: true });
    expect(invokeMock).toHaveBeenCalledWith('runtime_capabilities', undefined);
  });
});
