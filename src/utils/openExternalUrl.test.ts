// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  openExternalUrl: vi.fn(),
  supportsNativeCommands: vi.fn(),
}));

// Submodules, not the barrel: the .eml viewer reaches this file, and a barrel
// import would pull the whole service surface into its bundle (PERF-08).
vi.mock('@/services/backend/system', () => ({
  backendOpenExternalUrl: backend.openExternalUrl,
}));
vi.mock('@/services/backend/native', () => ({
  backendCommandsAvailable: backend.supportsNativeCommands,
  backendSupportsNativeCommands: backend.supportsNativeCommands,
}));

import { openExternalUrl, supportsNativeExternalUrls } from './openExternalUrl';
import { backendErrorKey } from '@/utils/backendErrors';

const browserOpen = vi.spyOn(window, 'open');

describe('openExternalUrl', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    backend.openExternalUrl.mockResolvedValue(undefined);
    backend.supportsNativeCommands.mockReturnValue(true);
    browserOpen.mockReset().mockReturnValue(null);
  });

  it('uses the native backend in the desktop runtime', async () => {
    await openExternalUrl('https://example.com/native');

    expect(backend.openExternalUrl).toHaveBeenCalledWith('https://example.com/native');
    expect(browserOpen).not.toHaveBeenCalled();
  });

  it('opens a new tab with a safe opener policy on the web', async () => {
    backend.supportsNativeCommands.mockReturnValue(false);
    const opened = { opener: window } as unknown as Window;
    browserOpen.mockReturnValue(opened);

    await openExternalUrl('https://example.com/web');

    expect(browserOpen).toHaveBeenCalledWith(
      'https://example.com/web',
      '_blank',
      'noopener,noreferrer',
    );
    expect(opened.opener).toBeNull();
    expect(backend.openExternalUrl).not.toHaveBeenCalled();
  });

  /* The rejection is interpolated into a translated toast, so it names a key. */
  it('rejects with a translatable key when the browser blocks the popup', async () => {
    backend.supportsNativeCommands.mockReturnValue(false);

    const error = await openExternalUrl('https://example.com/blocked')
      .catch((thrown: unknown) => thrown);

    expect(backendErrorKey(error)).toBe('backend.errors.browserWindowBlocked');
    expect((error as Error).message).not.toContain('browser window was blocked');
  });

  it('exposes the same runtime decision used by the opener', () => {
    expect(supportsNativeExternalUrls()).toBe(true);
    backend.supportsNativeCommands.mockReturnValue(false);
    expect(supportsNativeExternalUrls()).toBe(false);
  });
});
