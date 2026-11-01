import { setCommandFailureListener } from '@/services/backend/core';
import { backendErrorKey, backendErrorStringParam } from '@/utils/backendErrors';
import { recordFrontendDiagnostic } from '@/utils/diagnostics';

/** A command failing in a loop says nothing new after its first line. */
const COMMAND_FAILURE_REPEAT_MS = 10_000;

/**
 * Logs failed commands as `command.failed`, beside the clicks that caused
 * them. The UI often shows only a translated headline; the log is where the
 * key and Rust's own message survive.
 */
export function installCommandFailureDiagnostics(): void {
  const lastLogged = new Map<string, number>();
  setCommandFailureListener((command, error) => {
    // Reporting the diagnostics command's own failure would feed itself.
    if (command === 'record_frontend_diagnostic') return;
    const key = backendErrorKey(error);
    const message = backendErrorStringParam(error, 'message') ?? (key ? null : error.message);
    const detail = [
      `command=${command}`,
      `key=${key ?? 'none'}`,
      message ? `message=${message.replace(/\s+/g, ' ').slice(0, 300)}` : null,
    ].filter(Boolean).join(' ');
    const now = Date.now();
    if (now - (lastLogged.get(detail) ?? -Infinity) < COMMAND_FAILURE_REPEAT_MS) return;
    // Bounded: distinct failures are few, but a session can run for weeks.
    if (lastLogged.size >= 100) lastLogged.clear();
    lastLogged.set(detail, now);
    recordFrontendDiagnostic('command.failed', detail, undefined, 'warn');
  });
}
