import type { RuntimeCapabilitiesDto as WireRuntimeCapabilitiesDto } from './generated/app/commands/runtime/RuntimeCapabilitiesDto';
// Auto-split from services/backend.ts — IPC plumbing.
import { perfEnabled } from '@/utils/perf';
import { BackendError, translateBackendError } from '@/utils/backendErrors';
import { backendCommandsAvailable, sendCommand } from '@/services/backend/transport';

export { backendSupportsNativeCommands } from './native';
export { backendCommandsAvailable } from '@/services/backend/transport';

export {
  backendErrorKey,
  backendErrorStringParam,
  translateBackendError,
} from '@/utils/backendErrors';

/**
 * What the shell this build runs in can actually do.
 *
 * Asked once at startup instead of inferred from the user agent: the same
 * frontend bundle runs on the desktop app, Android and iOS, and the pieces
 * that differ (tray, autostart, the WASM plugin runtime)
 * are absent rather than merely unused on mobile.
 */
export type BackendRuntimeCapabilities = WireRuntimeCapabilitiesDto;


function isTauriRuntimeError(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    lower.includes('not running inside tauri runtime') ||
    lower.includes('__tauri_internals__') ||
    lower.includes('tauri api')
  );
}

export function normalizeInvokeError(error: unknown): Error {
  const commandErrorMessage = translateBackendError(error);
  if (commandErrorMessage) {
    const normalized = new Error(commandErrorMessage);
    normalized.name = 'BackendCommandError';
    normalized.cause = error;
    return normalized;
  }
  const message = error instanceof Error ? error.message : String(error);
  if (isTauriRuntimeError(message)) {
    return new BackendError('backend.errors.desktopUnavailable');
  }
  return error instanceof Error ? error : new Error(message);
}


export async function invoke<T>(command: string, payload?: Record<string, unknown>): Promise<T> {
  if (!backendCommandsAvailable()) {
    throw new BackendError('backend.errors.desktopUnavailable');
  }

  const startedAt = perfEnabled() ? performance.now() : 0;
  try {
    const result = await sendCommand<T>(command, payload);
    if (startedAt > 0) {
      console.info(`[perf] invoke:${command} ${(performance.now() - startedAt).toFixed(1)}ms`);
    }
    return result;
  } catch (error) {
    if (startedAt > 0) {
      console.info(`[perf] invoke:${command}:error ${(performance.now() - startedAt).toFixed(1)}ms`);
    }
    throw normalizeInvokeError(error);
  }
}
