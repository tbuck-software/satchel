import { t } from '@/i18n/runtime';
import { translateBackendError } from '@/utils/backendErrors';

/**
 * Labels for the connection store's backend error channel.
 *
 * Two things have to be legible before a failure is worth showing: which part
 * of the app stopped working, and why. The subsystem comes from a static table
 * — a key assembled from the source string (`errors.source.${source}`) would
 * drop out of the i18n analysis and ship as a raw key. The cause comes from the
 * backend's structured error; raw text survives only as a last resort, because
 * a cryptic message still beats a hidden one.
 *
 * The table holds message keys, not text: a table of translated strings would
 * be built once at import and keep the language the app started in. Same
 * reasoning as `outboxLabels.ts`, which does this for the outbox queue.
 */

/** Every subsystem that reports into `connection.reportBackendError`. */
export const BACKEND_ERROR_SOURCES = [
  'threads',
  'messages',
  'mailboxes',
  'accounts',
  'connection',
  'sync',
  'outbox',
] as const;

export type BackendErrorSource = (typeof BACKEND_ERROR_SOURCES)[number];

const SOURCE_KEYS: Record<string, string> = {
  threads: 'errors.source.threads',
  messages: 'errors.source.messages',
  mailboxes: 'errors.source.mailboxes',
  accounts: 'errors.source.accounts',
  connection: 'errors.source.connection',
  sync: 'errors.source.sync',
  outbox: 'errors.source.outbox',
};

const UNKNOWN_SOURCE_KEY = 'errors.source.unknown';

/** Names the failing subsystem. An unlisted source stays generic, never raw. */
export function backendErrorSourceLabel(source?: string | null): string {
  return t((source && SOURCE_KEYS[source]) || UNKNOWN_SOURCE_KEY);
}

function rawText(error: unknown): string {
  if (error === null || error === undefined) return '';
  if (error instanceof Error) return error.message.trim();
  if (typeof error === 'string') return error.trim();
  return String(error).trim();
}

/**
 * Turns whatever a subsystem reported into a cause a person can read.
 *
 * Commands answer with a structured `backend.errors.*` key, which is what gets
 * translated here. UX-11 replaces the raw fallback with `transportErrorLabel()`
 * for auth, TLS and DNS failures; the call sites do not change.
 */
export function backendErrorLabel(error?: unknown): string {
  const translated = translateBackendError(error);
  if (translated) return translated;
  return rawText(error) || t('errors.backendUnknown');
}
