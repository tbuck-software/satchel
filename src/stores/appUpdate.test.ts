import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useAppUpdateStore } from './appUpdate';

const backend = vi.hoisted(() => ({
  check: vi.fn(), download: vi.fn(), install: vi.fn(),
}));
vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: () => true,
  backendCheckAppUpdate: backend.check,
  backendDownloadAppUpdate: backend.download,
  backendInstallAppUpdate: backend.install,
}));

describe('app update store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

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
    backend.check.mockResolvedValue({ channel: 'nightly', version: '0.1.1-nightly.42' });
    backend.download.mockImplementation(async (report: (percent: number) => void) => report(100));
    backend.install.mockResolvedValue(undefined);
    const update = useAppUpdateStore();
    await update.checkForUpdates();
    expect(update.version).toBe('0.1.1-nightly.42');
    expect(update.channel).toBe('nightly');
    await update.activate();
    expect(backend.download).toHaveBeenCalledOnce();
    // A periodic check must not replace the verified package awaiting restart.
    await update.checkForUpdates();
    expect(backend.check).toHaveBeenCalledOnce();
    expect(update.status).toBe('ready');
    await update.activate();
    expect(backend.install).toHaveBeenCalledOnce();
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
