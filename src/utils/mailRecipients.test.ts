import { describe, expect, it } from 'vitest';
import { preferredExternalReplyAddress } from './mailRecipients';

describe('preferredExternalReplyAddress', () => {
  it('uses Reply-To when it is external', () => {
    expect(
      preferredExternalReplyAddress(
        'replies@example.net',
        'sender@example.com',
        ['me@example.com'],
      ),
    ).toBe('replies@example.net');
  });

  it('falls back to From when Reply-To belongs to the user', () => {
    expect(
      preferredExternalReplyAddress(
        'ME@example.com',
        'sender@example.com',
        ['me@example.com'],
      ),
    ).toBe('sender@example.com');
  });

  it('returns null when every candidate belongs to the user', () => {
    expect(
      preferredExternalReplyAddress('me@example.com', 'alias@example.com', [
        'me@example.com',
        'alias@example.com',
      ]),
    ).toBeNull();
  });
});
