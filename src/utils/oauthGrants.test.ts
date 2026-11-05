import { describe, expect, it } from 'vitest';
import { hasOptionalGrant } from './oauthGrants';

describe('hasOptionalGrant', () => {
  it('matches Google scopes exactly', () => {
    const scopes = ['https://www.googleapis.com/auth/gmail.modify', 'https://www.googleapis.com/auth/calendar.readonly'];
    expect(hasOptionalGrant('gmail', 'calendar', scopes)).toBe(true);
    expect(hasOptionalGrant('gmail', 'contacts', scopes)).toBe(false);
  });

  it('matches Outlook calendar access with or without the Graph prefix', () => {
    expect(hasOptionalGrant('outlook', 'calendar', ['https://graph.microsoft.com/Calendars.ReadWrite'])).toBe(true);
    expect(hasOptionalGrant('outlook', 'calendar', ['calendars.readwrite'])).toBe(true);
    expect(hasOptionalGrant('outlook', 'calendar', ['Calendars.Read', 'Mail.ReadWrite'])).toBe(false);
    expect(hasOptionalGrant('outlook', 'calendar', null)).toBe(false);
    // Outlook contacts are part of the sign-in, not an optional grant.
    expect(hasOptionalGrant('outlook', 'contacts', ['Contacts.Read'])).toBe(false);
  });
});
