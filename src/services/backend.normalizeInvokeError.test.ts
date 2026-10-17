import { describe, expect, it } from 'vitest';
import { applyLocalePreference } from '@/i18n/runtime';
import { backendErrorKey, backendErrorStringParam, normalizeInvokeError } from './backend';

describe('normalizeInvokeError', () => {
  it('maps Tauri runtime failures to an actionable backend-unavailable message', () => {
    for (const raw of [
      'window.__TAURI_INTERNALS__ is undefined',
      'Cannot read properties of undefined (reading __TAURI_INTERNALS__)',
      'plugin tauri api not available',
    ]) {
      const normalized = normalizeInvokeError(new Error(raw));
      // Carries the key, so a German reader gets a German sentence.
      expect(backendErrorKey(normalized)).toBe('backend.errors.desktopUnavailable');
      expect(normalized.message).toContain('desktop backend is unavailable');
    }
  });

  it('passes domain errors through with their message intact', () => {
    expect(normalizeInvokeError(new Error('smtp send failed')).message).toBe(
      'smtp send failed',
    );
    // Same identity: no re-wrap that would break instanceof checks upstream.
    const original = new Error('disk full');
    expect(normalizeInvokeError(original)).toBe(original);
  });

  it('wraps non-Error rejections into Error instances', () => {
    for (const raw of ['string failure', 42, null, { code: 'EACCES' }]) {
      const normalized = normalizeInvokeError(raw);
      expect(normalized).toBeInstanceOf(Error);
      expect(normalized.message).toBe(String(raw));
    }
  });

  it('translates structured command errors with their interpolation params', () => {
    const normalized = normalizeInvokeError({
      key: 'backend.errors.folderExists',
      params: { path: 'Archive/2026' },
    });

    expect(normalized).toBeInstanceOf(Error);
    expect(normalized.name).toBe('BackendCommandError');
    expect(normalized.message).toBe('A folder named "Archive/2026" already exists.');
    expect(backendErrorKey(normalized)).toBe('backend.errors.folderExists');
  });

  it('uses the active locale when decoding a command error', async () => {
    await applyLocalePreference('de', false);
    try {
      expect(normalizeInvokeError('backend.errors.pgpSmimeConflict').message).toBe(
        'PGP und S/MIME können nicht in derselben Nachricht verwendet werden.',
      );
    } finally {
      await applyLocalePreference('en', false);
    }
  });

  it('decodes JSON-wrapped payloads but rejects malformed contracts', () => {
    expect(normalizeInvokeError(JSON.stringify({
      key: 'backend.errors.folderNameTooLong',
      params: { max: 200 },
    })).message).toBe('The folder name cannot be longer than 200 characters.');

    expect(normalizeInvokeError({
      key: 'backend.errors.folderExists',
      params: ['not', 'a', 'map'],
    }).message).toBe('[object Object]');
  });

  it('decodes persisted sync and queue failures', () => {
    const queuedFailure = JSON.stringify({
      key: 'backend.errors.queueActionFailed',
      params: { message: 'remote move rejected' },
    });

    expect(normalizeInvokeError(JSON.stringify({
      key: 'backend.errors.accountSyncFailed',
      params: { account: 'person@example.test', message: 'provider refused' },
    })).message).toBe('Sync for person@example.test failed: provider refused');
    expect(normalizeInvokeError(queuedFailure).message).toBe(
      'The queued action failed: remote move rejected',
    );
    expect(backendErrorStringParam(queuedFailure, 'message')).toBe('remote move rejected');
  });
});
