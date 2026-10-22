import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAppUpdateStore } from './appUpdate';
import { useToast } from '@/composables/useToast';

const backend = vi.hoisted(() => ({
  check: vi.fn(), download: vi.fn(), install: vi.fn(), preferences: vi.fn(), setChannel: vi.fn(),
}));
vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: () => true,
  backendCheckAppUpdate: backend.check,
  backendDownloadAppUpdate: backend.download,
  backendInstallAppUpdate: backend.install,
  backendGetAppUpdatePreferences: backend.preferences,
  backendSetAppUpdateChannel: backend.setChannel,
}));

describe('app update store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    backend.preferences.mockResolvedValue({ supported: true, channel: 'stable' });
    backend.check.mockResolvedValue({ channel: 'stable', version: null });
    backend.download.mockResolvedValue(undefined);
    backend.install.mockResolvedValue(undefined);
    backend.setChannel.mockResolvedValue(undefined);
  });

  afterEach(() => { useToast().clear(); vi.useRealTimers(); });

  it('moves from available through download progress to restart-ready', async () => {
    const restart = vi.fn().mockResolvedValue(undefined);
    const update = useAppUpdateStore();

    update.offerUpdate({
      version: '1.2.3',
      download: async (reportProgress) => {
        reportProgress(0);
        reportProgress(42.4);
        reportProgress(100);
      },
      restart,
    });

    expect(update.status).toBe('available');
    expect(update.visible).toBe(true);

    await update.activate();

    expect(update.status).toBe('ready');
    expect(update.progress).toBe(100);

    await update.activate();
    expect(restart).toHaveBeenCalledOnce();
  });

  it('keeps progress monotonic and clamps values to a percentage', async () => {
    const update = useAppUpdateStore();
    update.offerUpdate({
      download: async (reportProgress) => {
        reportProgress(57);
        reportProgress(12);
        reportProgress(120);
      },
      restart: vi.fn(),
    });

    await update.downloadUpdate();

    expect(update.progress).toBe(100);
    expect(update.status).toBe('ready');
  });

  it('returns a failed download to a retryable state', async () => {
    const download = vi
      .fn()
      .mockRejectedValueOnce(new Error('network unavailable'))
      .mockResolvedValueOnce(undefined);
    const update = useAppUpdateStore();
    update.offerUpdate({ download, restart: vi.fn() });

    await update.activate();
    expect(update.status).toBe('failed');
    expect(update.error).toBe('network unavailable');

    await update.activate();
    expect(download).toHaveBeenCalledTimes(2);
    expect(update.status).toBe('ready');
  });

  it('retries a failed restart without downloading the package again', async () => {
    const download = vi.fn().mockResolvedValue(undefined);
    const restart = vi
      .fn()
      .mockRejectedValueOnce(new Error('restart blocked'))
      .mockResolvedValueOnce(undefined);
    const update = useAppUpdateStore();
    update.offerUpdate({ download, restart });

    await update.activate();
    await update.activate();
    expect(update.status).toBe('failed');

    await update.activate();
    expect(download).toHaveBeenCalledOnce();
    expect(restart).toHaveBeenCalledTimes(2);
  });
  it('connects a checked backend offer to download and installation', async () => {
    backend.preferences.mockResolvedValue({ supported: true, channel: 'nightly' });
    backend.check.mockResolvedValue({ channel: 'nightly', version: '0.1.1-nightly.42' });
    backend.download.mockImplementation(async (report: (percent: number) => void) => report(100));
    backend.install.mockResolvedValue(undefined);
    const update = useAppUpdateStore();
    await update.checkForUpdates();
    expect(update.version).toBe('0.1.1-nightly.42');
    expect(update.channel).toBe('nightly');
    expect(backend.download).toHaveBeenCalledOnce();
    // A periodic check must not replace the verified package awaiting restart.
    await update.checkForUpdates();
    expect(backend.check).toHaveBeenCalledOnce();
    expect(update.status).toBe('ready');
    await update.activate();
    expect(backend.install).toHaveBeenCalledOnce();
  });

  it('downloads in the background and offers a persistent restart action that can be dismissed', async () => {
    backend.check.mockResolvedValue({ channel: 'stable', version: '0.1.2' });
    const update = useAppUpdateStore();
    await update.checkForUpdates();
    expect(update.status).toBe('ready');
    const toast = useToast().state.toasts.at(-1)!;
    expect(toast.timeout).toBe(0);
    expect(toast.message).toContain('0.1.2');
    expect(toast.action?.label).toBe('Restart now');
    expect(backend.install).not.toHaveBeenCalled();
    useToast().dismiss(toast.id);
    expect(update.status).toBe('ready');
    await update.activate();
    expect(backend.install).toHaveBeenCalledOnce();
  });

  it('persists the channel before checking its feed and discards an old ready offer', async () => {
    const update = useAppUpdateStore();
    await update.loadPreferences();
    const oldRestart = vi.fn();
    update.offerUpdate({ download: vi.fn(), restart: oldRestart });
    await update.downloadUpdate();
    expect(useToast().state.toasts).toHaveLength(1);
    expect(await update.setChannel('nightly')).toBe(true);
    expect(backend.setChannel).toHaveBeenCalledWith('nightly');
    expect(backend.setChannel.mock.invocationCallOrder[0]).toBeLessThan(backend.check.mock.invocationCallOrder[0]!);
    expect(update.channel).toBe('nightly');
    expect(update.status).toBe('hidden');
    expect(useToast().state.toasts).toHaveLength(0);
    expect(oldRestart).not.toHaveBeenCalled();
  });

  it('keeps the selected channel and downloaded update if saving the choice fails', async () => {
    const update = useAppUpdateStore();
    await update.loadPreferences();
    update.offerUpdate({ version: '0.1.2', download: vi.fn(), restart: vi.fn() });
    await update.downloadUpdate();
    backend.setChannel.mockRejectedValueOnce(new Error('cannot save'));
    expect(await update.setChannel('nightly')).toBe(false);
    expect(update.channel).toBe('stable');
    expect(update.status).toBe('ready');
    expect(update.checkError).toBe('cannot save');
    expect(backend.check).not.toHaveBeenCalled();
  });

  it('leaves source builds untouched and does not contact a release feed', async () => {
    backend.preferences.mockResolvedValue({ supported: false, channel: 'stable' });
    const update = useAppUpdateStore();
    await update.checkForUpdates();
    expect(backend.check).not.toHaveBeenCalled();
    expect(await update.setChannel('nightly')).toBe(false);
    expect(backend.setChannel).not.toHaveBeenCalled();
  });

  it('checks automatically after startup and every six hours until stopped', async () => {
    vi.useFakeTimers();
    const update = useAppUpdateStore();
    update.start();
    await vi.advanceTimersByTimeAsync(8_000);
    expect(backend.check).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(6 * 60 * 60_000);
    expect(backend.check).toHaveBeenCalledTimes(2);
    update.stop();
    await vi.advanceTimersByTimeAsync(6 * 60 * 60_000);
    expect(backend.check).toHaveBeenCalledTimes(2);
  });

  it('keeps an unavailable private feed retryable without fabricating an offer', async () => {
    backend.check.mockRejectedValueOnce(new Error('feed unavailable'));
    backend.check.mockResolvedValueOnce({ channel: 'stable', version: null });
    const update = useAppUpdateStore();
    await update.checkForUpdates();
    expect(update.visible).toBe(false);
    expect(update.checkError).toBe('feed unavailable');
    await update.checkForUpdates();
    expect(update.checkError).toBeNull();
    expect(update.checked).toBe(true);
  });

});
