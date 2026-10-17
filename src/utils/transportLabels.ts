import { t } from '@/i18n/runtime';
import {
  backendErrorKey,
  backendErrorStringParam,
  translateBackendError,
} from '@/utils/backendErrors';

/**
 * One classifier for the transport failures a person can trigger.
 *
 * Account setup, the command palette, the server search and the outbox all
 * catch the same handful of failures out of the same Rust stack, and each of
 * them used to answer differently: a raw error chain in the wizard, an empty
 * `catch {}` in the palette, an outbox-only sentence in the queue. The classes
 * below are the shared vocabulary. A surface may still pick its own wording per
 * class — `outboxErrorLabel` does — but nobody classifies twice.
 *
 * Unknown errors fall through unchanged. A cryptic message beats a hidden one;
 * that is the documented behaviour of `outboxLabels.ts` and it is kept here.
 *
 * The tables hold message keys, not text: a table of translated strings would
 * be built once at import and keep the language the app started in.
 */

export type TransportErrorClass =
  | 'rateLimit'
  | 'quota'
  | 'appPasswordRequired'
  | 'oauthConfigMissing'
  | 'oauthExpired'
  | 'oauthCancelled'
  | 'credentialsRejected'
  | 'tls'
  | 'dns'
  | 'connectionRefused'
  | 'recipientRejected'
  | 'offline';

/**
 * Markers in the order they are tested. Order is the whole design: the Gmail
 * refusal for a two-factor account says "application-specific password
 * required" *and* reads as a credential failure, and "connection refused" is a
 * closed port rather than a missing network. The narrower class comes first.
 */
const PATTERNS: { errorClass: TransportErrorClass; markers: string[] }[] = [
  {
    errorClass: 'rateLimit',
    markers: ['rate limit', 'rate-limit', 'too many requests', 'throttl'],
  },
  {
    errorClass: 'quota',
    markers: ['quota', 'limit exceeded'],
  },
  {
    errorClass: 'appPasswordRequired',
    markers: [
      'application-specific password',
      'application specific password',
      'app password',
      'app-specific password',
      'two-factor',
      'two factor',
      '2-step',
      'basic authentication is disabled',
      'web login required',
    ],
  },
  {
    errorClass: 'oauthConfigMissing',
    markers: ['missing satchel_'],
  },
  {
    errorClass: 'oauthExpired',
    markers: [
      'invalid_grant',
      'invalid_token',
      'token has been expired',
      'token expired',
      'refresh token',
    ],
  },
  {
    errorClass: 'oauthCancelled',
    markers: [
      'access_denied',
      'authorization cancelled',
      'authorization canceled',
      'user cancelled',
      'user canceled',
      'consent denied',
    ],
  },
  {
    errorClass: 'credentialsRejected',
    markers: [
      'authenticationfailed',
      'authentication failed',
      'authenticate failed',
      'invalid credentials',
      'username or password',
      'login failed',
      'auth failed',
      'unauthorized',
    ],
  },
  {
    errorClass: 'tls',
    markers: [
      'tls',
      'ssl',
      'certificate',
      'handshake',
      'unknownissuer',
      'notvalidfor',
      'self-signed',
      'self signed',
    ],
  },
  {
    errorClass: 'dns',
    markers: [
      'dns error',
      'failed to lookup address',
      'no such host',
      'name or service not known',
      'nodename nor servname',
      'getaddrinfo',
      'name resolution',
      'could not resolve',
    ],
  },
  {
    errorClass: 'connectionRefused',
    markers: [
      'connection refused',
      'econnrefused',
      'connection reset',
      'no route to host',
      'actively refused',
      'connection aborted',
    ],
  },
  {
    errorClass: 'recipientRejected',
    markers: ['no such user', 'recipient rejected', 'invalid recipient', 'user unknown'],
  },
  {
    errorClass: 'offline',
    markers: ['unreachable', 'timed out', 'timeout', 'offline', 'network', 'connection'],
  },
];

/** The neutral wording. A surface with a better sentence overrides its class. */
const CLASS_KEYS: Record<TransportErrorClass, string> = {
  rateLimit: 'errors.transport.rateLimit',
  quota: 'errors.transport.quota',
  appPasswordRequired: 'errors.transport.appPasswordRequired',
  oauthConfigMissing: 'errors.oauth.missingConfig',
  oauthExpired: 'errors.transport.oauthExpired',
  oauthCancelled: 'errors.transport.oauthCancelled',
  credentialsRejected: 'errors.transport.credentialsRejected',
  tls: 'errors.transport.tls',
  dns: 'errors.transport.dns',
  connectionRefused: 'errors.transport.connectionRefused',
  recipientRejected: 'errors.transport.recipientRejected',
  offline: 'errors.transport.offline',
};

const QUEUE_FAILURE_KEY = 'backend.errors.queueActionFailed';

/** Whatever a caller caught, as text. */
export function transportErrorText(error: unknown): string {
  if (error === null || error === undefined) return '';
  if (error instanceof Error) return error.message.trim();
  if (typeof error === 'string') return error.trim();
  return String(error).trim();
}

/**
 * The provider diagnostic a queued failure carries, if any.
 *
 * `queueActionFailed` is an envelope: the sentence it translates to says only
 * that the queue gave up, while the reason a person can act on sits in its
 * `message` parameter.
 */
function diagnosticOf(error: unknown): string | null {
  return backendErrorKey(error) === QUEUE_FAILURE_KEY
    ? backendErrorStringParam(error, 'message')
    : null;
}

/** Name the class of failure, or `null` when it is not one we recognise. */
export function transportErrorClass(error?: unknown): TransportErrorClass | null {
  const raw = transportErrorText(error);
  if (!raw) return null;
  const lower = (diagnosticOf(error) ?? raw).toLowerCase();
  return PATTERNS.find(({ markers }) => markers.some((marker) => lower.includes(marker)))
    ?.errorClass ?? null;
}

/** Gmail and Outlook answer for themselves; anything else stays generic. */
export function transportProviderName(error?: unknown): string {
  const lower = transportErrorText(error).toLowerCase();
  if (lower.includes('gmail') || lower.includes('google')) return 'Gmail';
  if (lower.includes('outlook') || lower.includes('microsoft')) return 'Outlook';
  return t('errors.transport.genericProvider');
}

/**
 * Turn a transport failure into a sentence someone can act on.
 *
 * A structured backend key wins — Rust already named that failure. Otherwise
 * the plain error chain is classified, and an unrecognised one is returned
 * unchanged so it can still be read.
 *
 * `overrides` lets a surface answer a class in its own voice without repeating
 * that precedence; the outbox does this for the classes it phrases as queue
 * outcomes rather than as setup advice.
 */
export function transportErrorLabel(
  error?: unknown,
  overrides?: Partial<Record<TransportErrorClass, string>>,
): string {
  const raw = transportErrorText(error);
  if (!raw) return '';

  const diagnostic = diagnosticOf(error);
  if (backendErrorKey(error) && diagnostic === null) {
    return translateBackendError(error) ?? raw;
  }
  if (diagnostic) {
    const translatedDiagnostic = translateBackendError(diagnostic);
    if (translatedDiagnostic) return translatedDiagnostic;
  }

  const errorClass = transportErrorClass(error);
  if (errorClass) {
    const key = overrides?.[errorClass] ?? CLASS_KEYS[errorClass];
    return t(key, { provider: transportProviderName(error) });
  }
  return translateBackendError(raw) ?? raw;
}

/**
 * The translated cause for a message that already has a headline, or `''`.
 *
 * A toast that appends a raw Rust chain is worse than one that says nothing,
 * so an unrecognised failure contributes no cause at all.
 */
export function transportErrorCause(error?: unknown): string {
  if (!transportErrorText(error)) return '';
  if (backendErrorKey(error)) return transportErrorLabel(error);
  return transportErrorClass(error) ? transportErrorLabel(error) : '';
}
