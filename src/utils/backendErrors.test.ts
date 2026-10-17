import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LocaleMessageValue } from 'vue-i18n';
import { createAppI18n } from '@/i18n/create';
import { backendErrorKey, translateBackendError } from './backendErrors';

function activate(messages: Record<string, LocaleMessageValue>) {
  createAppI18n({ en: messages, de: {} }, 'en');
}

beforeEach(() => {
  // Missing keys are the point of these tests; keep vue-i18n quiet about them.
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

afterEach(() => {
  activate({});
  vi.restoreAllMocks();
});

describe('backend error translation', () => {
  it('translates a known key with its parameters', () => {
    activate({ backend: { errors: { folderExists: 'Folder {path} exists', unexpected: 'Unexpected' } } });

    expect(translateBackendError({ key: 'backend.errors.folderExists', params: { path: 'A' } }))
      .toBe('Folder A exists');
  });

  it('falls back to the generic failure when a subset catalogue lacks the key', () => {
    activate({ backend: { errors: { unexpected: 'Something went wrong.' } } });

    expect(translateBackendError('backend.errors.webhookUrlInvalid')).toBe('Something went wrong.');
    expect(backendErrorKey('backend.errors.webhookUrlInvalid')).toBe('backend.errors.webhookUrlInvalid');
  });

  it('leaves the key visible when even the fallback is missing', () => {
    activate({});

    expect(translateBackendError('backend.errors.webhookUrlInvalid')).toBe('backend.errors.webhookUrlInvalid');
  });

  it('ignores errors that are not command errors', () => {
    activate({ backend: { errors: { unexpected: 'Unexpected' } } });

    expect(translateBackendError(new Error('plain failure'))).toBeNull();
  });
});
