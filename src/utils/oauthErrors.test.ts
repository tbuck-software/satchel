import { describe, expect, it } from 'vitest';
import { authErrorLabel } from './oauthErrors';

describe('authErrorLabel', () => {
  it('does not claim that invalid_grant always means an explicit revocation', () => {
    expect(
      authErrorLabel(
        'oauth refresh failed with status 400 Bad Request: {"error":"invalid_grant","error_description":"Token has been expired or revoked."}',
      ),
    ).toBe(
      'Google no longer accepts the stored sign-in (expired or revoked). Please sign in again.',
    );
  });

  it('keeps the existing guidance for missing local OAuth configuration', () => {
    expect(authErrorLabel('missing SATCHEL_GMAIL_CLIENT_ID')).toBe(
      'OAuth configuration is missing. Please reinstall Satchel or check the local .env file.',
    );
  });

  it('passes through unknown errors', () => {
    expect(authErrorLabel('network unavailable')).toBe('network unavailable');
  });
});
