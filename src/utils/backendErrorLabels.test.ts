import { describe, expect, it } from 'vitest';
import {
  BACKEND_ERROR_SOURCES,
  backendErrorLabel,
  backendErrorSourceLabel,
} from './backendErrorLabels';

describe('backendErrorSourceLabel', () => {
  it('has a sentence for every subsystem that can report', () => {
    for (const source of BACKEND_ERROR_SOURCES) {
      expect(backendErrorSourceLabel(source)).not.toContain('errors.source');
    }
  });

  it('stays generic for a source nobody registered', () => {
    expect(backendErrorSourceLabel('plugins')).toBe(backendErrorSourceLabel(null));
    expect(backendErrorSourceLabel(null)).toBe('Something in Satchel stopped working');
  });
});

describe('backendErrorLabel', () => {
  it('translates the structured error the commands answer with', () => {
    const error = new Error('{"key":"backend.errors.unexpected"}');

    expect(backendErrorLabel(error)).not.toContain('backend.errors');
  });

  it('names a sign-in that changed under a running request, also inside a sync failure', () => {
    const reload = 'backend.errors.oauthReloadRequired';
    const sentence = backendErrorLabel(reload);

    expect(sentence).not.toContain('backend.errors');
    expect(sentence).not.toBe(backendErrorLabel('backend.errors.unexpected'));
    expect(sentence).not.toBe(backendErrorLabel('backend.errors.queueTemporary'));
    // Mailbox sync wraps the cause in its per-account failure.
    const syncFailure = JSON.stringify({
      key: 'backend.errors.accountSyncFailed',
      params: { account: 'me@example.test', accountId: 'account', message: reload },
    });
    expect(backendErrorLabel(syncFailure)).toBe(`Sync for me@example.test failed: ${sentence}`);
    // A provider's own wording stays verbatim.
    const plain = JSON.stringify({
      key: 'backend.errors.accountSyncFailed',
      params: { account: 'me@example.test', message: 'imap login failed' },
    });
    expect(backendErrorLabel(plain)).toBe('Sync for me@example.test failed: imap login failed');
  });

  it('keeps an untranslatable message rather than hiding it', () => {
    expect(backendErrorLabel(new Error('database is locked'))).toBe('database is locked');
  });

  it('falls back to a named cause when there is nothing to show', () => {
    expect(backendErrorLabel('  ')).toBe('Unknown backend error');
    expect(backendErrorLabel(null)).toBe('Unknown backend error');
  });
});
