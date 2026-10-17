import { appCacheDir, join } from '@tauri-apps/api/path';
import { readFile, remove, writeFile } from '@tauri-apps/plugin-fs';

export function isAndroidContentUri(path: string): boolean {
  return path.startsWith('content://');
}

async function temporaryPath(extension: string): Promise<string> {
  const cache = await appCacheDir();
  const suffix = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return join(cache, `satchel-transfer-${suffix}${extension}`);
}

export async function withWritableExportPath<T>(
  selectedPath: string,
  extension: string,
  writer: (localPath: string) => Promise<T>,
): Promise<T> {
  if (!isAndroidContentUri(selectedPath)) return writer(selectedPath);

  const localPath = await temporaryPath(extension);
  try {
    const result = await writer(localPath);
    await writeFile(selectedPath, await readFile(localPath));
    return result;
  } finally {
    await remove(localPath).catch(() => undefined);
  }
}

export async function withReadableImportPath<T>(
  selectedPath: string,
  extension: string,
  reader: (localPath: string) => Promise<T>,
): Promise<T> {
  if (!isAndroidContentUri(selectedPath)) return reader(selectedPath);

  const localPath = await temporaryPath(extension);
  try {
    await writeFile(localPath, await readFile(selectedPath));
    return await reader(localPath);
  } finally {
    await remove(localPath).catch(() => undefined);
  }
}
