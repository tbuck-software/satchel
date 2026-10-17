import { describe, expect, it } from 'vitest';
import { applyLocalePreference } from '@/i18n/runtime';
import { outboxErrorLabel } from './outboxLabels';

describe('outboxErrorLabel backend keys', () => {
  it('translates persisted backend error keys with the active locale', async () => {
    expect(outboxErrorLabel('backend.errors.pgpSmimeConflict')).toBe(
      'PGP and S/MIME cannot be used in the same message.',
    );

    await applyLocalePreference('de', false);
    try {
      expect(outboxErrorLabel('backend.errors.pgpSmimeConflict')).toBe(
        'PGP und S/MIME können nicht in derselben Nachricht verwendet werden.',
      );
    } finally {
      await applyLocalePreference('en', false);
    }
  });
});
