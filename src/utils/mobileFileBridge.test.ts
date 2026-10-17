import { beforeEach, describe, expect, it, vi } from 'vitest';

const fs = vi.hoisted(() => ({
  readFile: vi.fn(),
  writeFile: vi.fn(),
  remove: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-fs', () => fs);
vi.mock('@tauri-apps/api/path', () => ({
  appCacheDir: vi.fn(async () => '/cache'),
  join: vi.fn(async (...parts: string[]) => parts.join('/')),
}));

import { withReadableImportPath, withWritableExportPath } from './mobileFileBridge';

describe('Android content URI bridge', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fs.remove.mockResolvedValue(undefined);
  });

  it('copies a local backend export into the selected content URI', async () => {
    const bytes = new Uint8Array([1, 2, 3]);
    fs.readFile.mockResolvedValue(bytes);
    const writer = vi.fn(async () => 'done');

    await expect(withWritableExportPath('content://document/export', '.mbox', writer)).resolves.toBe('done');
    expect(writer).toHaveBeenCalledWith(expect.stringContaining('/cache/satchel-transfer-'));
    expect(fs.writeFile).toHaveBeenCalledWith('content://document/export', bytes);
    expect(fs.remove).toHaveBeenCalledOnce();
  });

  it('stages a selected content URI before a Rust path-based import', async () => {
    const bytes = new Uint8Array([4, 5]);
    fs.readFile.mockResolvedValue(bytes);
    const reader = vi.fn(async () => 2);

    await expect(withReadableImportPath('content://document/import', '.json', reader)).resolves.toBe(2);
    expect(fs.writeFile).toHaveBeenCalledWith(expect.stringContaining('/cache/satchel-transfer-'), bytes);
    expect(reader).toHaveBeenCalledWith(expect.stringContaining('/cache/satchel-transfer-'));
    expect(fs.remove).toHaveBeenCalledOnce();
  });

  it('passes ordinary desktop paths through unchanged', async () => {
    const writer = vi.fn(async () => 'ok');
    await withWritableExportPath('/tmp/export.mbox', '.mbox', writer);
    expect(writer).toHaveBeenCalledWith('/tmp/export.mbox');
    expect(fs.readFile).not.toHaveBeenCalled();
  });
});
