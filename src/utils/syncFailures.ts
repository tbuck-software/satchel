import { t } from '@/i18n/runtime';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { backendErrorKey, backendErrorStringParam } from '@/utils/backendErrors';
import { transportErrorClass, type TransportErrorClass } from '@/utils/transportLabels';

/** Classes a new password fixes; an expired OAuth grant reconnects instead. */
const PASSWORD_CLASSES: ReadonlySet<TransportErrorClass> = new Set([
  'credentialsRejected',
  'appPasswordRequired',
]);

/** Classes a fresh OAuth sign-in fixes, for an account that has no password. */
const SIGN_IN_CLASSES: ReadonlySet<TransportErrorClass> = new Set([
  'oauthExpired',
  'oauthCancelled',
  'credentialsRejected',
]);

/**
 * Classes that point at the account's server settings: a host name that does
 * not resolve, or a TLS setup the server does not speak. Retrying changes
 * nothing until the settings do. (A rejected certificate has its own banner.)
 */
const SETTINGS_CLASSES: ReadonlySet<TransportErrorClass> = new Set(['dns', 'tls']);

/**
 * What the banner offers for a failure beyond syncing again:
 * a new password, a new OAuth sign-in, or the account's settings.
 */
export type SyncRemedy = 'password' | 'signIn' | 'accountSettings' | 'retry';

/** The parts of an account the classification needs. */
export interface SyncFailureAccount {
  id: string;
  email: string;
  provider: string;
}

export interface SyncFailure {
  /** Translated at render time, so it follows the current language. */
  label: string;
  /**
   * Set for an account-level failure only. A folder failure names its account
   * by address and stays `null` here, so it never counts as an outage.
   */
  accountId: string | null;
  account: string;
  /** What the server said, verbatim; for a bug report, not for the headline. */
  serverText: string;
  passwordRejected: boolean;
  /** When this account's run of failed syncs began (RFC 3339), if known. */
  since: string | null;
  remedy: SyncRemedy;
  /** The account the remedy acts on; folder failures resolve it by address. */
  remedyAccountId: string | null;
}

function isPasswordProvider(provider: string): boolean {
  return provider === 'imap' || provider === 'pop3';
}

/** From this long on, a failure is an outage and the banner says since when. */
export const OUTAGE_AFTER_MS = 60 * 60 * 1000;

export function isOutage(failure: Pick<SyncFailure, 'since'>, now = Date.now()): boolean {
  const began = failure.since ? Date.parse(failure.since) : Number.NaN;
  return Number.isFinite(began) && now - began >= OUTAGE_AFTER_MS;
}

/**
 * One entry of `run_imap_sync`'s `failures`: an encoded
 * `backend.errors.accountSyncFailed` with the account and the provider's text,
 * or a `backend.errors.folderSyncFailed` that names the account by address.
 * `accounts` are the configured mail accounts; they say who signs in with a
 * password and resolve a folder failure's address to its account.
 */
export function describeSyncFailure(raw: string, accounts: readonly SyncFailureAccount[]): SyncFailure {
  const key = backendErrorKey(raw);
  const keyed = key === 'backend.errors.accountSyncFailed';
  const folderKeyed = key === 'backend.errors.folderSyncFailed';
  const accountId = keyed ? backendErrorStringParam(raw, 'accountId') : null;
  const account = (keyed || folderKeyed ? backendErrorStringParam(raw, 'account') : null) ?? '';
  const serverText = (keyed || folderKeyed ? backendErrorStringParam(raw, 'message') : null) ?? raw;
  const owner = accountId
    ? accounts.find((entry) => entry.id === accountId)
    : folderKeyed && account
      ? accounts.find((entry) => entry.email.toLowerCase() === account.toLowerCase())
      : undefined;
  const errorClass = transportErrorClass(serverText);
  const passwordRejected = keyed
    && owner !== undefined
    && isPasswordProvider(owner.provider)
    && errorClass !== null
    && PASSWORD_CLASSES.has(errorClass);
  let remedy: SyncRemedy = 'retry';
  if (passwordRejected) remedy = 'password';
  else if (owner && errorClass && !isPasswordProvider(owner.provider) && SIGN_IN_CLASSES.has(errorClass)) {
    remedy = 'signIn';
  } else if (owner && errorClass && SETTINGS_CLASSES.has(errorClass)) remedy = 'accountSettings';
  return {
    label: passwordRejected
      ? t('errors.banner.passwordRejected', { account })
      : backendErrorLabel(raw),
    accountId,
    account,
    serverText,
    passwordRejected,
    since: keyed ? backendErrorStringParam(raw, 'since') : null,
    remedy,
    remedyAccountId: remedy === 'retry' ? null : owner?.id ?? null,
  };
}
