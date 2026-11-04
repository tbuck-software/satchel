import { backendErrorKey, backendErrorStringParam } from '@/utils/backendErrors';

/** The key `tls_trust::handshake_error` encodes a rejected certificate under. */
export const CERTIFICATE_REJECTED_KEY = 'backend.errors.tlsCertificateRejected';

export type CertificateRejectionReason = 'expired' | 'notYetValid' | 'nameMismatch' | 'unknownIssuer' | 'other';

const REASONS: ReadonlySet<string> = new Set(['expired', 'notYetValid', 'nameMismatch', 'unknownIssuer', 'other']);

/** What the mail server presented when its certificate was rejected. */
export interface CertificateRejection {
  protocol: string;
  host: string;
  port: string;
  reason: CertificateRejectionReason;
  /** Common name of the certificate's subject, e.g. the cluster node behind a load balancer. */
  subject: string | null;
  issuer: string | null;
  /** RFC 3339. */
  notBefore: string | null;
  /** RFC 3339. */
  notAfter: string | null;
  fingerprint: string;
  /** Expiry is the only fault, so a time-limited exception may be offered. */
  exceptionAllowed: boolean;
}

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null;
}

function fromPayload(payload: unknown): CertificateRejection | null {
  if (typeof payload !== 'object' || payload === null) return null;
  const { key, params } = payload as { key?: unknown; params?: unknown };
  if (key !== CERTIFICATE_REJECTED_KEY || typeof params !== 'object' || params === null) return null;
  const values = params as Record<string, unknown>;
  const host = asString(values.host);
  const fingerprint = asString(values.fingerprint);
  if (!host || !fingerprint) return null;
  const reason = asString(values.reason);
  return {
    protocol: asString(values.protocol) ?? 'imap',
    host,
    port: asString(values.port) ?? '',
    reason: reason && REASONS.has(reason) ? reason as CertificateRejectionReason : 'other',
    subject: asString(values.subject),
    issuer: asString(values.issuer),
    notBefore: asString(values.notBefore),
    notAfter: asString(values.notAfter),
    fingerprint,
    exceptionAllowed: values.exceptionAllowed === true,
  };
}

/**
 * The encoded rejection inside a string, wherever it sits. Provider errors
 * reach the account as text, and a caller on the way may have put a prefix in
 * front of it; the JSON object itself is intact.
 */
function embedded(text: string): CertificateRejection | null {
  const start = text.indexOf(`{"key":"${CERTIFICATE_REJECTED_KEY}"`);
  if (start < 0) return null;
  let depth = 0;
  let inString = false;
  for (let index = start; index < text.length; index += 1) {
    const char = text[index];
    if (inString) {
      if (char === '\\') index += 1;
      else if (char === '"') inString = false;
    } else if (char === '"') {
      inString = true;
    } else if (char === '{') {
      depth += 1;
    } else if (char === '}') {
      depth -= 1;
      if (depth === 0) {
        try {
          return fromPayload(JSON.parse(text.slice(start, index + 1)));
        } catch {
          return null;
        }
      }
    }
  }
  return null;
}

/**
 * The certificate rejection behind a sync failure, if that is what it was.
 * Takes an encoded `accountSyncFailed` (the cause sits in its `message`), the
 * bare rejection, or any text that embeds one.
 */
export function certificateRejection(raw: string | null | undefined): CertificateRejection | null {
  if (!raw) return null;
  if (backendErrorKey(raw) === 'backend.errors.accountSyncFailed') {
    return certificateRejection(backendErrorStringParam(raw, 'message'));
  }
  return embedded(raw);
}

/**
 * The account and rejection behind one `accountSyncFailed` entry of a sync
 * run. Only these become notices; anything else stays an ordinary failure.
 */
export function accountCertificateRejection(raw: string): { accountId: string; rejection: CertificateRejection } | null {
  if (backendErrorKey(raw) !== 'backend.errors.accountSyncFailed') return null;
  const accountId = backendErrorStringParam(raw, 'accountId');
  const rejection = certificateRejection(raw);
  return accountId && rejection ? { accountId, rejection } : null;
}

/** Two rejections are the same notice when the reason and the certificate match. */
export function rejectionSignature(rejection: CertificateRejection): string {
  return `${rejection.host}|${rejection.reason}|${rejection.fingerprint}`;
}
