import type { CommandError } from '@/services/backend/generated/app/commands/error/CommandError';
import { t, te } from '@/i18n/runtime';

// The decoder also accepts frontend Error causes with arbitrary interpolation values.
type BackendCommandErrorPayload = Omit<CommandError, 'params'> & { params?: Record<string, unknown> };

function commandErrorPayload(value: unknown): BackendCommandErrorPayload | null {
  if (typeof value === 'string') {
    if (value.startsWith('backend.errors.')) return { key: value };
    if (!value.trimStart().startsWith('{')) return null;
    try {
      return commandErrorPayload(JSON.parse(value));
    } catch {
      return null;
    }
  }

  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null;
  const candidate = value as Record<string, unknown>;
  if (typeof candidate.key !== 'string' || !candidate.key.startsWith('backend.errors.')) {
    return null;
  }
  const params = candidate.params;
  if (params === undefined) return { key: candidate.key };
  if (typeof params !== 'object' || params === null || Array.isArray(params)) return null;
  return { key: candidate.key, params: params as Record<string, unknown> };
}

function findCommandErrorPayload(error: unknown): BackendCommandErrorPayload | null {
  const direct = commandErrorPayload(error);
  if (direct) return direct;
  if (!(error instanceof Error)) return null;
  return commandErrorPayload(error.cause) ?? commandErrorPayload(error.message);
}

const FALLBACK_KEY = 'backend.errors.unexpected';

/**
 * A frontend failure that names a message key instead of a sentence — the same
 * `{ key, params }` contract Rust already uses. `message` stays translated so
 * the ~75 call sites interpolating `error.message` keep working.
 */
export class BackendError extends Error {
  readonly key: string;

  constructor(key: string, params?: Record<string, unknown>) {
    super(te(key) ? t(key, params) : key);
    this.key = key;
    this.name = 'BackendError';
    this.cause = { key, params };
  }
}

/**
 * Translate a command error object or an encoded error stored in a string field.
 *
 * The viewer and tray catalogues carry only the keys their commands emit, so a
 * key outside that set reads as the generic failure instead of a raw key.
 */
export function translateBackendError(error: unknown): string | null {
  const payload = findCommandErrorPayload(error);
  if (!payload) return null;
  if (!te(payload.key) && te(FALLBACK_KEY)) return t(FALLBACK_KEY);
  return t(payload.key, payload.params);
}

/** Return the stable message key without coupling callers to Tauri's wrapper shape. */
export function backendErrorKey(error: unknown): string | null {
  return findCommandErrorPayload(error)?.key ?? null;
}

/** Read a string interpolation value without making callers parse persisted errors again. */
export function backendErrorStringParam(error: unknown, name: string): string | null {
  const value = findCommandErrorPayload(error)?.params?.[name];
  return typeof value === 'string' ? value : null;
}
