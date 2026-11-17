// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { writeClipboardText } from './clipboard';

const clipboard = vi.hoisted(() => ({ native: false, writeText: vi.fn() }));
vi.mock('@/services/backend/native', () => ({ backendSupportsNativeCommands: () => clipboard.native }));
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ writeText: clipboard.writeText }));

afterEach(() => {
  clipboard.native = false;
  delete document.documentElement.dataset.runtimeMobile;
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe('clipboard writes', () => {
  it('uses the desktop plugin after asynchronous body loading', async () => {
    clipboard.native = true;
    await writeClipboardText('Complete message');
    expect(clipboard.writeText).toHaveBeenCalledWith('Complete message');
  });

  it.each([false, true])('uses the browser clipboard on the web and mobile shell, native=%s', async native => {
    clipboard.native = native;
    document.documentElement.dataset.runtimeMobile = String(native);
    const writeText = vi.fn();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    await writeClipboardText('Complete message');
    expect(writeText).toHaveBeenCalledWith('Complete message');
    expect(clipboard.writeText).not.toHaveBeenCalled();
  });
});
