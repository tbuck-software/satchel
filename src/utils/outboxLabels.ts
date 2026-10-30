import { t, tc } from '@/i18n/runtime';
import { backendErrorKey } from '@/utils/backendErrors';
import {
  transportErrorLabel,
  type TransportErrorClass,
} from '@/utils/transportLabels';

/**
 * Labels for the outbox queue.
 *
 * The queue speaks the backend's language: statuses like `pending`, action types
 * like `send_message`, and raw provider errors such as
 * `gmail send skipped: rate limited for another 730s`. None of that belongs in
 * front of a person, so it is translated here.
 *
 * The tables hold message keys, not text: a table of translated strings would
 * be built once at import and keep the language the app started in.
 */

export type OutboxTone = 'danger' | 'warning' | 'accent' | 'neutral';

/** Statuses `list_visible` can return, in the order they matter to the user. */
export const OUTBOX_STATUS_ORDER = [
  'delivered',
  'delivery_unknown',
  'failed',
  'sending',
  'scheduled',
  'pending',
] as const;

const STATUS_KEYS: Record<string, string> = {
  delivered: 'outbox.status.delivered',
  delivery_unknown: 'outbox.status.deliveryUnknown',
  failed: 'outbox.status.failed',
  sending: 'outbox.status.sending',
  scheduled: 'outbox.status.scheduled',
  pending: 'outbox.status.pending',
};

const STATUS_TONES: Record<string, OutboxTone> = {
  delivered: 'warning',
  delivery_unknown: 'warning',
  failed: 'danger',
  sending: 'accent',
  // A send waiting for its time is the queue working, not a warning; only an
  // overdue one turns amber, and the view decides that.
  scheduled: 'neutral',
  pending: 'neutral',
};

const ACTION_KEYS: Record<string, string> = {
  send_message: 'outbox.action.sendMessage',
  remote_mutation: 'outbox.action.remoteMutation',
};

const OVERDUE_KEY = 'outbox.time.overdueSince';

export function outboxStatusLabel(status: string): string {
  const key = STATUS_KEYS[status];
  return key ? t(key) : status;
}

export function outboxStatusTone(status: string): OutboxTone {
  return STATUS_TONES[status] ?? 'neutral';
}

export function outboxActionLabel(actionType: string): string {
  const key = ACTION_KEYS[actionType];
  return key ? t(key) : actionType;
}

export function outboxActionCanBeCancelled(actionType: string, status: string): boolean {
  // Only mail that is known not to have reached the provider may be discarded.
  return (actionType === 'remote_mutation' && status === 'failed') || actionType === 'send_message'
    && ['pending', 'scheduled', 'failed'].includes(status);
}

/** Terminal delivery warnings may be acknowledged, never retried or discarded. */
export function outboxActionCanBeDismissed(actionType: string, status: string): boolean {
  return actionType === 'send_message'
    && (status === 'delivered' || status === 'delivery_unknown');
}

/**
 * A send may be repeated unless it is known to have reached the provider.
 * `delivery_unknown` without a started submission is a crash before the first
 * byte went out — there is nothing to duplicate.
 */
export function outboxActionCanBeRetried(
  actionType: string,
  status: string,
  submissionStartedAt?: string | null,
): boolean {
  if (actionType !== 'send_message') return actionType === 'remote_mutation' && status === 'failed';
  if (status === 'sending' || status === 'delivered') return false;
  if (status === 'delivery_unknown') return !submissionStartedAt;
  return true;
}

/** Mail whose text is worth rescuing into the composer. */
export function outboxActionCanBeReopenedAsDraft(actionType: string, status: string): boolean {
  return actionType === 'send_message'
    && (status === 'failed' || status === 'delivery_unknown');
}

/**
 * A queued send whose draft changed after it was queued cannot become a draft
 * again. It is still queued, and the Outbox is where it can be read or stopped.
 */
export function queuedDraftNeedsOutbox(error: unknown): boolean {
  return backendErrorKey(error) === 'backend.errors.queuedDraftNotRestorable';
}

export function outboxVisibleCount(pendingSends: number, failedSends: number): number {
  return Math.max(0, pendingSends) + Math.max(0, failedSends);
}

export function outboxRetryLabel(actionType: string, status: string): string {
  if (actionType !== 'send_message') return t('common.retry');
  return t(status === 'failed' ? 'outbox.retry.again' : 'outbox.retry.now');
}

export function outboxAttemptsLabel(count: number): string {
  return tc('outbox.attempts', count);
}

/** Which timestamp explains the row best — when the item will move next. */
export function outboxTimeLabelKey(
  status: string,
  value?: string | null,
  now = Date.now(),
): string {
  if (status === 'scheduled') {
    const timestamp = value ? new Date(value).getTime() : Number.NaN;
    return Number.isFinite(timestamp) && timestamp <= now
      ? OVERDUE_KEY
      : 'outbox.time.scheduledFor';
  }
  if (status === 'delivered') return 'outbox.time.delivered';
  if (status === 'delivery_unknown') return 'outbox.time.lastAttempt';
  if (status === 'failed') return 'outbox.time.nextRetry';
  if (status === 'sending') return 'outbox.time.started';
  return 'outbox.time.queued';
}

export function outboxTimeLabel(status: string, value?: string | null, now = Date.now()): string {
  return t(outboxTimeLabelKey(status, value, now));
}

/** Asks the question directly instead of comparing rendered labels. */
export function outboxIsOverdue(status: string, value?: string | null, now = Date.now()): boolean {
  return outboxTimeLabelKey(status, value, now) === OVERDUE_KEY;
}

/**
 * The queue's own wording for the classes it has a better sentence for.
 *
 * Everything else — an app password that is missing, a certificate that does
 * not verify — falls through to the neutral transport wording rather than
 * being flattened into "no connection".
 */
const ERROR_KEYS: Partial<Record<TransportErrorClass, string>> = {
  rateLimit: 'outbox.error.rateLimit',
  quota: 'outbox.error.quota',
  credentialsRejected: 'outbox.error.auth',
  oauthExpired: 'outbox.error.auth',
  connectionRefused: 'outbox.error.network',
  offline: 'outbox.error.network',
  recipientRejected: 'outbox.error.recipientRejected',
};

/**
 * Turns a backend error into a sentence someone can act on. Unknown errors fall
 * through unchanged — a cryptic message beats a hidden one.
 */
export function outboxErrorLabel(error?: string | null): string {
  return transportErrorLabel(error?.trim(), ERROR_KEYS);
}
