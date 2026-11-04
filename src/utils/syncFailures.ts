import { t } from '@/i18n/runtime';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { backendErrorKey, backendErrorStringParam } from '@/utils/backendErrors';
import { transportErrorClass, type TransportErrorClass } from '@/utils/transportLabels';

/** Classes a new password fixes; an expired OAuth grant reconnects instead. */
const PASSWORD_CLASSES: ReadonlySet<TransportErrorClass> = new Set([
  'credentialsRejected',
  'appPasswordRequired',
]);

export interface SyncFailure {
  /** Translated at render time, so it follows the current language. */
  label: string;
  accountId: string | null;
  account: string;
  /** What the server said, verbatim; for a bug report, not for the headline. */
  serverText: string;
  passwordRejected: boolean;
  /** When this account's run of failed syncs began (RFC 3339), if known. */
  since: string | null;
}

/** From this long on, a failure is an outage and the banner says since when. */
export const OUTAGE_AFTER_MS = 60 * 60 * 1000;

export function isOutage(failure: Pick<SyncFailure, 'since'>, now = Date.now()): boolean {
  const began = failure.since ? Date.parse(failure.since) : Number.NaN;
  return Number.isFinite(began) && now - began >= OUTAGE_AFTER_MS;
}

/**
 * One entry of `run_imap_sync`'s `failures`: an encoded
 * `backend.errors.accountSyncFailed` with the account and the provider's text.
 * `passwordAccounts` holds the accounts that sign in with a password at all.
 */
export function describeSyncFailure(raw: string, passwordAccounts: ReadonlySet<string>): SyncFailure {
  const keyed = backendErrorKey(raw) === 'backend.errors.accountSyncFailed';
  const accountId = keyed ? backendErrorStringParam(raw, 'accountId') : null;
  const account = (keyed ? backendErrorStringParam(raw, 'account') : null) ?? '';
  const serverText = (keyed ? backendErrorStringParam(raw, 'message') : null) ?? raw;
  const errorClass = transportErrorClass(serverText);
  const passwordRejected = accountId !== null
    && passwordAccounts.has(accountId)
    && errorClass !== null
    && PASSWORD_CLASSES.has(errorClass);
  return {
    label: passwordRejected
      ? t('errors.banner.passwordRejected', { account })
      : backendErrorLabel(raw),
    accountId,
    account,
    serverText,
    passwordRejected,
    since: keyed ? backendErrorStringParam(raw, 'since') : null,
  };
}
