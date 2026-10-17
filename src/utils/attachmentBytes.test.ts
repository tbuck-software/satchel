import { beforeEach, describe, expect, it, vi } from 'vitest';

/*
 * PERF-12. `download_attachment` embedded every attachment as base64, however
 * large — a 50 MB file existed at once as the read buffer, a 67 MB string,
 * serde's JSON buffer and the JS string at twice that again. Above 4 MiB only
 * `cached_path` comes back now, and everything that wants bytes comes here.
 */

const harness = vi.hoisted(() => ({ readFile: vi.fn() }));

vi.mock('@tauri-apps/plugin-fs', () => ({ readFile: harness.readFile }));

import { attachmentBytes, encodeBase64 } from './attachmentBytes';

beforeEach(() => {
  harness.readFile.mockReset();
});

describe('attachment bytes', () => {
  it('decodes the payload when the backend sent one', async () => {
    const bytes = await attachmentBytes({ data_base64: 'AQID', cached_path: '/cache/a.bin' });

    expect(bytes).toEqual(new Uint8Array([1, 2, 3]));
    expect(harness.readFile).not.toHaveBeenCalled();
  });

  it('reads the cache file when the payload was too large to send', async () => {
    harness.readFile.mockResolvedValue(new Uint8Array([4, 5, 6]));

    const bytes = await attachmentBytes({ data_base64: null, cached_path: '/cache/big.pdf' });

    expect(bytes).toEqual(new Uint8Array([4, 5, 6]));
    expect(harness.readFile).toHaveBeenCalledWith('/cache/big.pdf');
  });

  it('says so when there is neither', async () => {
    await expect(attachmentBytes({ data_base64: null, cached_path: null }))
      .rejects.toMatchObject({ key: 'backend.errors.attachmentPayloadMissing' });
  });

  /** Forwarding re-encodes; `fromCharCode(...bytes)` would blow the stack. */
  it('re-encodes a payload larger than one argument list', () => {
    const large = new Uint8Array(200_000).fill(65);

    const encoded = encodeBase64(large);

    expect(encoded).toBe(btoa('A'.repeat(200_000)));
  });
});
