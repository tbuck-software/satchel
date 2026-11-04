import { describe, expect, it } from 'vitest';
import { CERTIFICATE_REJECTED_KEY, certificateRejection, rejectionSignature } from './certificateRejection';

const rejection = {
  key: CERTIFICATE_REJECTED_KEY,
  params: {
    protocol: 'imap',
    host: 'imap.example.test',
    port: '993',
    reason: 'expired',
    subject: 'imap-02.cluster.example.test',
    issuer: 'GEANT TLS RSA 1',
    notBefore: '2025-11-04T11:06:26Z',
    notAfter: '2026-11-04T11:06:26Z',
    fingerprint: '6D:39:A7:FC',
    exceptionAllowed: true,
  },
};

function accountFailure(message: string): string {
  return JSON.stringify({
    key: 'backend.errors.accountSyncFailed',
    params: { account: 'me@example.test', accountId: 'acc-work', message },
  });
}

describe('certificateRejection', () => {
  it('reads the rejection out of an account sync failure', () => {
    const parsed = certificateRejection(accountFailure(JSON.stringify(rejection)));

    expect(parsed).toEqual({
      protocol: 'imap',
      host: 'imap.example.test',
      port: '993',
      reason: 'expired',
      subject: 'imap-02.cluster.example.test',
      issuer: 'GEANT TLS RSA 1',
      notBefore: '2025-11-04T11:06:26Z',
      notAfter: '2026-11-04T11:06:26Z',
      fingerprint: '6D:39:A7:FC',
      exceptionAllowed: true,
    });
  });

  // A caller on the way may put its own words in front of the provider error.
  it('finds a rejection behind a prefix', () => {
    const parsed = certificateRejection(accountFailure(`folder sync failed: ${JSON.stringify(rejection)}; INBOX`));

    expect(parsed?.host).toBe('imap.example.test');
  });

  it('ignores every other failure', () => {
    expect(certificateRejection(accountFailure('imap tls handshake failed: reset by peer'))).toBeNull();
    expect(certificateRejection(accountFailure('{"key":"backend.errors.imapUidValidityMissing"}'))).toBeNull();
    expect(certificateRejection(null)).toBeNull();
  });

  it('refuses a rejection without the certificate it is about', () => {
    const broken = { ...rejection, params: { ...rejection.params, fingerprint: '' } };

    expect(certificateRejection(JSON.stringify(broken))).toBeNull();
  });

  it('never offers an exception the backend did not allow', () => {
    const strict = { ...rejection, params: { ...rejection.params, exceptionAllowed: 'yes' } };

    expect(certificateRejection(JSON.stringify(strict))?.exceptionAllowed).toBe(false);
  });

  it('reads an unknown reason as other', () => {
    const odd = { ...rejection, params: { ...rejection.params, reason: 'revoked' } };

    expect(certificateRejection(JSON.stringify(odd))?.reason).toBe('other');
  });

  it('tells two certificates apart', () => {
    const first = certificateRejection(JSON.stringify(rejection))!;
    const renewed = certificateRejection(JSON.stringify({ ...rejection, params: { ...rejection.params, fingerprint: 'AA:BB' } }))!;

    expect(rejectionSignature(first)).not.toBe(rejectionSignature(renewed));
  });
});
