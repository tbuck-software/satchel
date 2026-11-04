// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendListCertificateExceptions: vi.fn(async () => [] as unknown[]),
  backendTrustExpiredCertificate: vi.fn(),
  backendRevokeCertificateException: vi.fn(async () => true),
}));
vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  ...backend,
}));

import { useCertificateTrustStore } from './certificateTrust';
import { useConnectionStore } from './connection';

const exception = {
  account_id: 'acc-work',
  host: 'gwimap.example.test',
  fingerprint: '6D:39',
  expires_at: '2026-11-11T13:00:00Z',
  created_at: '2026-11-04T13:00:00Z',
};

function failure(accountId: string, reason = 'expired') {
  return JSON.stringify({
    key: 'backend.errors.accountSyncFailed',
    params: {
      account: 'tom@example.test',
      accountId,
      message: JSON.stringify({
        key: 'backend.errors.tlsCertificateRejected',
        params: { protocol: 'imap', host: 'gwimap.example.test', port: '993', reason, fingerprint: '6D:39', exceptionAllowed: true },
      }),
    },
  });
}

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

describe('certificate trust', () => {
  beforeEach(() => {
    window.localStorage.clear();
    (window as TauriWindow).__TAURI_INTERNALS__ = {};
    setActivePinia(createPinia());
    vi.clearAllMocks();
    backend.backendTrustExpiredCertificate.mockResolvedValue(exception);
  });

  it('knows the account behind each rejected certificate, from the live run only', () => {
    const connection = useConnectionStore();
    const trust = useCertificateTrustStore();
    connection.reportSyncFailures([failure('acc-work')]);

    expect(trust.problemFor('acc-work')?.rejection.host).toBe('gwimap.example.test');
    expect(trust.problemFor('acc-other')).toBeNull();

    connection.reportSyncFailures([]);
    expect(trust.problemFor('acc-work')).toBeNull();
  });

  it('trusts exactly the certificate it showed, hides the notice and syncs again', async () => {
    const connection = useConnectionStore();
    const trust = useCertificateTrustStore();
    connection.reportSyncFailures([failure('acc-work')]);

    await trust.trust('acc-work');

    expect(backend.backendTrustExpiredCertificate).toHaveBeenCalledWith({
      account_id: 'acc-work',
      host: 'gwimap.example.test',
      fingerprint: '6D:39',
    });
    expect(trust.visibleProblems).toEqual([]);
    expect(trust.exceptionFor('acc-work')).toEqual(exception);
    expect(connection.retryToken).toBe(1);
  });

  it('ends an exception and asks for the next run', async () => {
    const connection = useConnectionStore();
    const trust = useCertificateTrustStore();
    connection.reportSyncFailures([failure('acc-work')]);
    await trust.trust('acc-work');

    await trust.revoke('acc-work', 'gwimap.example.test');

    expect(backend.backendRevokeCertificateException).toHaveBeenCalledWith('acc-work', 'gwimap.example.test');
    expect(trust.exceptionFor('acc-work')).toBeNull();
    expect(connection.retryToken).toBe(2);
  });

  // Seven days later the same certificate may be rejected again; trusting it
  // once must not have hidden it for good.
  it('hides a trusted notice only until the next run reports', async () => {
    const connection = useConnectionStore();
    const trust = useCertificateTrustStore();
    connection.reportSyncFailures([failure('acc-work')]);
    trust.dismiss('acc-work');

    await trust.trust('acc-work');
    expect(trust.visibleProblems).toEqual([]);
    expect(window.localStorage.getItem('satchel.certificateNotices.dismissed.v1')).toBe('[]');

    connection.reportSyncFailures([]);
    connection.reportSyncFailures([failure('acc-work')]);
    expect(trust.visibleProblems.map((problem) => problem.accountId)).toEqual(['acc-work']);
  });

  it('shows the notice again once an exception is ended', async () => {
    const connection = useConnectionStore();
    const trust = useCertificateTrustStore();
    connection.reportSyncFailures([failure('acc-work')]);
    await trust.trust('acc-work');
    connection.reportSyncFailures([]);

    connection.reportSyncFailures([failure('acc-work')]);
    trust.dismiss('acc-work');
    await trust.revoke('acc-work', 'gwimap.example.test');
    connection.reportSyncFailures([failure('acc-work')]);

    expect(trust.visibleProblems.map((problem) => problem.accountId)).toEqual(['acc-work']);
  });

  it('leaves a rejection outside an account envelope to the ordinary failures', () => {
    const connection = useConnectionStore();
    const trust = useCertificateTrustStore();
    const bare = JSON.parse(failure('acc-work')).params.message as string;

    connection.reportSyncFailures([bare]);

    expect(trust.problems).toEqual([]);
  });
});
