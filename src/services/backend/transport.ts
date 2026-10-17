import { backendSupportsNativeCommands } from './native';

/**
 * The one seam between the frontend and a backend.
 *
 * `vite.config.ts` swaps this module for `./mock.ts` in mock builds. Nothing
 * above this file may branch on "are we mocking" — a store that takes a
 * different path under test proves nothing about the path it takes in
 * production, which is how a broken send survived a green suite.
 */
export function backendCommandsAvailable(): boolean {
  return backendSupportsNativeCommands();
}

export async function sendCommand<T>(
  command: string,
  payload?: Record<string, unknown>,
): Promise<T> {
  const mod = await import('@tauri-apps/api/core');
  return mod.invoke<T>(command, payload);
}
