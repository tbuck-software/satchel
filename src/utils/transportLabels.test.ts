import { describe, expect, it } from 'vitest';
import {
  transportErrorCause,
  transportErrorClass,
  transportErrorLabel,
  type TransportErrorClass,
} from './transportLabels';

/**
 * The wording of these inputs is the point: every one of them is a chain the
 * Rust side actually hands over, down to the case and the punctuation. A table
 * of invented messages would classify perfectly and prove nothing.
 */
const CHAINS: { chain: string; expected: TransportErrorClass }[] = [
  {
    chain: 'imap: login failed: [AUTHENTICATIONFAILED] Invalid credentials (Failure)',
    expected: 'credentialsRejected',
  },
  {
    chain: 'smtp: 535 5.7.8 Authentication failed: Bad username or password',
    expected: 'credentialsRejected',
  },
  {
    chain: 'imap: [ALERT] Application-specific password required: '
      + 'https://support.google.com/accounts/answer/185833',
    expected: 'appPasswordRequired',
  },
  {
    chain: 'imap: LOGIN failed: basic authentication is disabled for this tenant',
    expected: 'appPasswordRequired',
  },
  {
    chain: 'imap: connect: tls handshake: invalid peer certificate: UnknownIssuer',
    expected: 'tls',
  },
  {
    chain: 'smtp: connect: dns error: failed to lookup address information: '
      + 'nodename nor servname provided, or not known',
    expected: 'dns',
  },
  {
    chain: 'imap: connect: tcp connect error: Connection refused (os error 61)',
    expected: 'connectionRefused',
  },
  {
    chain: 'gmail send skipped: rate limited for another 730s',
    expected: 'rateLimit',
  },
  {
    chain: 'oauth refresh failed with status 400 Bad Request: '
      + '{"error":"invalid_grant","error_description":"Token has been expired or revoked."}',
    expected: 'oauthExpired',
  },
  {
    chain: 'oauth callback reported error=access_denied',
    expected: 'oauthCancelled',
  },
  {
    chain: 'smtp: 552 5.2.2 Mailbox quota exceeded',
    expected: 'quota',
  },
  {
    chain: 'imap: connect: tcp connect error: Network is unreachable (os error 51)',
    expected: 'offline',
  },
];

describe('transportErrorClass', () => {
  it.each(CHAINS)('reads $expected out of a real error chain', ({ chain, expected }) => {
    expect(transportErrorClass(chain)).toBe(expected);
  });

  it('prefers the app password over the credential failure it also reads as', () => {
    // Gmail answers a two-factor account with both statements at once. The
    // credential wording is true but useless; the app password is the next step.
    expect(transportErrorClass(
      '[AUTHENTICATIONFAILED] Invalid credentials — application-specific password required',
    )).toBe('appPasswordRequired');
  });

  it('does not name a class it cannot see', () => {
    expect(transportErrorClass('mime encode failed at part 3')).toBeNull();
    expect(transportErrorClass('')).toBeNull();
    expect(transportErrorClass(null)).toBeNull();
  });
});

describe('transportErrorLabel', () => {
  it('answers each class with its own sentence', () => {
    const sentences = new Set(CHAINS.map(({ chain }) => transportErrorLabel(chain)));
    expect(sentences.size).toBe(new Set(CHAINS.map((entry) => entry.expected)).size);
  });

  it('names the next step for the most common first-setup failure', () => {
    expect(transportErrorLabel('imap: [ALERT] Application-specific password required'))
      .toContain('app password');
  });

  it('tells a wrong password apart from a wrong port', () => {
    const credentials = transportErrorLabel('[AUTHENTICATIONFAILED] Invalid credentials');
    const port = transportErrorLabel('tcp connect error: Connection refused (os error 61)');
    expect(credentials).not.toBe(port);
    expect(port).toContain('port');
  });

  it('reads an error out of an Error, not just out of a string', () => {
    expect(transportErrorLabel(new Error('imap: connect: tls handshake failed')))
      .toBe(transportErrorLabel('imap: connect: tls handshake failed'));
  });

  it('passes an unknown chain through untouched', () => {
    expect(transportErrorLabel('mime encode failed at part 3'))
      .toBe('mime encode failed at part 3');
    expect(transportErrorLabel(null)).toBe('');
  });

  it('lets a structured backend key answer for itself', () => {
    expect(transportErrorLabel('backend.errors.pgpSmimeConflict'))
      .toBe('PGP and S/MIME cannot be used in the same message.');
  });

  it('digs the provider diagnostic out of a queued failure envelope', () => {
    const queued = JSON.stringify({
      key: 'backend.errors.queueActionFailed',
      params: { message: 'imap: connect: tls handshake: invalid peer certificate' },
    });
    expect(transportErrorClass(queued)).toBe('tls');
  });
});

describe('transportErrorCause', () => {
  it('contributes nothing when the chain cannot be named', () => {
    // A toast that appends a Rust chain is worse than one that says nothing.
    expect(transportErrorCause('mime encode failed at part 3')).toBe('');
    expect(transportErrorCause(undefined)).toBe('');
  });

  it('contributes the sentence when the chain can be named', () => {
    expect(transportErrorCause('tcp connect error: Connection refused'))
      .toBe(transportErrorLabel('tcp connect error: Connection refused'));
  });
});
