import { describe, expect, it } from 'vitest';
import { describeSyncFailure, type SyncFailureAccount } from './syncFailures';

const accounts: SyncFailureAccount[] = [
  { id: 'acc-imap', email: 'tom@example.test', provider: 'imap' },
  { id: 'acc-gmail', email: 'Tom@Gmail.example', provider: 'gmail' },
];

function accountFailure(accountId: string, account: string, message: string): string {
  return JSON.stringify({ key: 'backend.errors.accountSyncFailed', params: { account, accountId, message } });
}

function folderFailure(account: string, message: string): string {
  return JSON.stringify({ key: 'backend.errors.folderSyncFailed', params: { account, folder: 'Abuse', message } });
}

describe('describeSyncFailure remedy', () => {
  it('asks a password account for its password', () => {
    const failure = describeSyncFailure(
      accountFailure('acc-imap', 'tom@example.test', 'NO [AUTHENTICATIONFAILED] Invalid credentials'),
      accounts,
    );
    expect(failure.remedy).toBe('password');
    expect(failure.remedyAccountId).toBe('acc-imap');
  });

  it('signs an OAuth account in again', () => {
    const failure = describeSyncFailure(
      accountFailure('acc-gmail', 'tom@gmail.example', 'invalid_grant: Token has been expired or revoked'),
      accounts,
    );
    expect(failure.remedy).toBe('signIn');
    expect(failure.remedyAccountId).toBe('acc-gmail');
    expect(failure.passwordRejected).toBe(false);
  });

  it('opens the settings for a server name or TLS setup the server refuses', () => {
    expect(describeSyncFailure(
      accountFailure('acc-imap', 'tom@example.test', 'dns error: failed to lookup address information'),
      accounts,
    ).remedy).toBe('accountSettings');
    expect(describeSyncFailure(
      accountFailure('acc-imap', 'tom@example.test', 'tls handshake eof'),
      accounts,
    ).remedy).toBe('accountSettings');
  });

  it('resolves a folder failure to its account by address, without making it an account failure', () => {
    const failure = describeSyncFailure(folderFailure('tom@gmail.example', '401 Unauthorized'), accounts);
    expect(failure.accountId).toBeNull();
    expect(failure.remedy).toBe('signIn');
    expect(failure.remedyAccountId).toBe('acc-gmail');
    expect(failure.serverText).toBe('401 Unauthorized');
  });

  it('leaves everything else to syncing again', () => {
    for (const raw of [
      folderFailure('tom@example.test', 'NO [SERVERBUG] internal error'),
      accountFailure('acc-imap', 'tom@example.test', 'imap tcp connect timed out'),
      accountFailure('acc-unknown', 'gone@example.test', 'dns error'),
      'plain failure text',
    ]) {
      const failure = describeSyncFailure(raw, accounts);
      expect(failure.remedy).toBe('retry');
      expect(failure.remedyAccountId).toBeNull();
    }
  });
});
