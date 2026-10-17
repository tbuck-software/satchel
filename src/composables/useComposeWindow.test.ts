// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  openComposeWindow: vi.fn(),
  supportsNativeCommands: vi.fn(),
}));

vi.mock('@/services/backend', () => ({
  backendOpenComposeWindow: backend.openComposeWindow,
  backendCommandsAvailable: backend.supportsNativeCommands,
  backendSupportsNativeCommands: backend.supportsNativeCommands,
}));

import { useComposeWindow } from './useComposeWindow';
import { backendErrorKey } from '@/utils/backendErrors';

const router = {
  resolve: vi.fn((location: unknown) => ({ href: `/resolved/${JSON.stringify(location)}` })),
  push: vi.fn(async () => undefined),
};

const browserOpen = vi.spyOn(window, 'open');

beforeEach(() => {
  vi.clearAllMocks();
  delete document.documentElement.dataset.runtimeMobile;
  backend.openComposeWindow.mockResolvedValue(undefined);
  backend.supportsNativeCommands.mockReturnValue(true);
  browserOpen.mockReset().mockReturnValue({ opener: window } as Window);
});

describe('useComposeWindow', () => {
  it('opens a native compose window with the supplied draft', async () => {
    const composeWindow = useComposeWindow(router);

    await expect(composeWindow.openNative('draft-7')).resolves.toBe(true);

    expect(backend.openComposeWindow).toHaveBeenCalledWith('draft-7');
    expect(router.resolve).not.toHaveBeenCalled();
    expect(browserOpen).not.toHaveBeenCalled();
  });

  it('reports that native opening is unavailable on the web', async () => {
    backend.supportsNativeCommands.mockReturnValue(false);
    const composeWindow = useComposeWindow(router);

    await expect(composeWindow.openNative()).resolves.toBe(false);

    expect(backend.openComposeWindow).not.toHaveBeenCalled();
  });

  it('resolves the standalone route for the web fallback', async () => {
    backend.supportsNativeCommands.mockReturnValue(false);
    const composeWindow = useComposeWindow(router);

    await composeWindow.open('draft-8');

    expect(router.resolve).toHaveBeenCalledWith({
      name: 'compose',
      query: { standalone: '1', draftId: 'draft-8' },
    });
    expect(browserOpen).toHaveBeenCalledWith(
      '/resolved/{"name":"compose","query":{"standalone":"1","draftId":"draft-8"}}',
      '_blank',
      'noopener,noreferrer',
    );
  });

  it('routes in place on mobile instead of asking for a second window', async () => {
    document.documentElement.dataset.runtimeMobile = 'true';
    const composeWindow = useComposeWindow(router);

    expect(composeWindow.supportsNative()).toBe(false);
    await composeWindow.open('draft-9');

    expect(router.push).toHaveBeenCalledWith({
      name: 'compose',
      query: { standalone: '1', draftId: 'draft-9' },
    });
    expect(backend.openComposeWindow).not.toHaveBeenCalled();
    expect(browserOpen).not.toHaveBeenCalled();
  });

  it('propagates a failed mobile navigation instead of reporting success', async () => {
    document.documentElement.dataset.runtimeMobile = 'true';
    const failure = new Error('route guard rejected');
    router.push.mockRejectedValueOnce(failure);
    const composeWindow = useComposeWindow(router);

    await expect(composeWindow.open()).rejects.toBe(failure);
  });

  it('propagates native errors to the caller', async () => {
    const error = new Error('window failed');
    backend.openComposeWindow.mockRejectedValue(error);
    const composeWindow = useComposeWindow(router);

    await expect(composeWindow.open()).rejects.toBe(error);
    expect(router.resolve).not.toHaveBeenCalled();
  });

  /*
   * The rejection reaches the user through `compose.window.popOutFailed`,
   * which interpolates `error.message`. An English sentence there landed in a
   * German toast, so the failure names a key the catalogue can answer.
   */
  it('rejects a blocked browser popup with a translatable key', async () => {
    backend.supportsNativeCommands.mockReturnValue(false);
    browserOpen.mockReturnValue(null);
    const composeWindow = useComposeWindow(router);

    const error = await composeWindow.open().catch((thrown: unknown) => thrown);

    expect(backendErrorKey(error)).toBe('backend.errors.browserWindowBlocked');
    expect((error as Error).message).not.toContain('browser window was blocked');
  });
});
