// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  native: vi.fn(),
  save: vi.fn(),
  writeTextFile: vi.fn(),
  createObjectURL: vi.fn(),
  revokeObjectURL: vi.fn(),
  click: vi.fn(),
}));

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: harness.native,
  backendSupportsNativeCommands: harness.native,
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: harness.save }));
vi.mock('@tauri-apps/plugin-fs', () => ({ writeTextFile: harness.writeTextFile }));

import { saveTextExport } from './saveTextExport';

const createElement = vi.spyOn(document, 'createElement');
let anchor: HTMLAnchorElement;

beforeEach(() => {
  vi.clearAllMocks();
  harness.native.mockReturnValue(true);
  harness.save.mockResolvedValue('/tmp/export.txt');
  harness.writeTextFile.mockResolvedValue(undefined);
  harness.createObjectURL.mockReturnValue('blob:export');
  harness.revokeObjectURL.mockImplementation(() => undefined);
  harness.click.mockImplementation(() => undefined);
  Object.defineProperty(URL, 'createObjectURL', {
    configurable: true,
    value: harness.createObjectURL,
  });
  Object.defineProperty(URL, 'revokeObjectURL', {
    configurable: true,
    value: harness.revokeObjectURL,
  });
  anchor = { href: '', download: '', click: harness.click } as unknown as HTMLAnchorElement;
  createElement.mockReturnValue(anchor);
});

describe('saveTextExport', () => {
  it('saves through the native dialog with the extension filter', async () => {
    await expect(saveTextExport('subject.md', '# body', 'text/markdown', 'md'))
      .resolves.toBe('/tmp/export.txt');

    expect(harness.save).toHaveBeenCalledWith({
      defaultPath: 'subject.md',
      filters: [{ name: 'MD', extensions: ['md'] }],
    });
    expect(harness.writeTextFile).toHaveBeenCalledWith('/tmp/export.txt', '# body');
  });

  it('returns false without writing when the native dialog is cancelled', async () => {
    harness.save.mockResolvedValue(null);

    await expect(saveTextExport('subject.txt', 'body', 'text/plain', 'txt')).resolves.toBe(false);

    expect(harness.writeTextFile).not.toHaveBeenCalled();
  });

  it('downloads through a browser anchor and revokes the object URL', async () => {
    harness.native.mockReturnValue(false);

    await expect(saveTextExport('subject.txt', 'body', 'text/plain', 'txt')).resolves.toBe(true);

    expect(harness.createObjectURL).toHaveBeenCalledWith(expect.any(Blob));
    const blob = harness.createObjectURL.mock.calls[0]?.[0] as Blob;
    expect(blob.type).toBe('text/plain;charset=utf-8');
    expect(anchor.href).toBe('blob:export');
    expect(anchor.download).toBe('subject.txt');
    expect(harness.click).toHaveBeenCalledOnce();
    expect(harness.revokeObjectURL).toHaveBeenCalledWith('blob:export');
  });

  it('revokes the browser URL and propagates download errors', async () => {
    harness.native.mockReturnValue(false);
    const error = new Error('download failed');
    harness.click.mockImplementation(() => { throw error; });

    await expect(saveTextExport('subject.txt', 'body', 'text/plain', 'txt')).rejects.toBe(error);
    expect(harness.revokeObjectURL).toHaveBeenCalledWith('blob:export');
  });

  it('propagates native write errors', async () => {
    const error = new Error('write failed');
    harness.writeTextFile.mockRejectedValue(error);

    await expect(saveTextExport('subject.txt', 'body', 'text/plain', 'txt')).rejects.toBe(error);
  });
});
