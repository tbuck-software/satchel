import type { BackendDownloadAttachmentResult } from '@/types/messages';
import { BackendError } from '@/utils/backendErrors';

/**
 * The bytes of a downloaded attachment, whichever way the backend sent them.
 *
 * Above 4 MiB `data_base64` is `null` and only `cached_path` comes back —
 * handing a 50 MB file through the IPC bridge as base64 cost roughly seven
 * times its size in live memory. `cached_path` is always set, so this cannot
 * fail for want of a source.
 */
export async function attachmentBytes(
  data: Pick<BackendDownloadAttachmentResult, 'data_base64' | 'cached_path'>,
): Promise<Uint8Array> {
  if (data.data_base64) {
    return Uint8Array.from(atob(data.data_base64), (character) => character.charCodeAt(0));
  }
  if (!data.cached_path) {
    throw new BackendError('backend.errors.attachmentPayloadMissing');
  }
  const { readFile } = await import('@tauri-apps/plugin-fs');
  return readFile(data.cached_path);
}

/**
 * Bytes back to base64, in chunks.
 *
 * `String.fromCharCode(...bytes)` on a multi-megabyte array overflows the
 * argument list; the chunk size is small enough to stay well under it.
 */
export function encodeBase64(bytes: Uint8Array): string {
  const CHUNK = 0x8000;
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + CHUNK));
  }
  return btoa(binary);
}
