import { describe, expect, it } from 'vitest';
import { isOwnEmail, participantDisplayName } from './mailIdentity';

const accounts = [
  { displayName: 'Sample User', email: 'sample.user@example.com' },
];

describe('mail identity', () => {
  it('uses the configured account name for an own address with a missing header name', () => {
    expect(participantDisplayName(accounts, {
      name: 'Sample',
      email: 'SAMPLE.USER@example.com',
    })).toBe('Sample User');
  });

  it('recognizes own messages by normalized address instead of a magic display name', () => {
    expect(isOwnEmail(accounts, ' Sample.User@EXAMPLE.com ')).toBe(true);
    expect(isOwnEmail(accounts, 'mara@example.com')).toBe(false);
  });
});
