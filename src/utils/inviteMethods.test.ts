import { describe, expect, it } from 'vitest';
import { inviteAcceptsResponse, isInvitation } from './inviteMethods';

describe('invite methods', () => {
  it('does not take an answer to an invitation for an invitation', () => {
    expect(isInvitation({ method: 'REPLY' })).toBe(false);
    expect(isInvitation({ method: ' reply ' })).toBe(false);
    expect(isInvitation({ method: 'COUNTER' })).toBe(false);
    expect(isInvitation({ method: 'REFRESH' })).toBe(false);
    expect(isInvitation({ method: 'DECLINECOUNTER' })).toBe(false);
    expect(isInvitation({ method: 'REQUEST' })).toBe(true);
    expect(isInvitation({ method: 'PUBLISH' })).toBe(true);
    expect(isInvitation({ method: null })).toBe(true);
  });

  it('offers an answer only where one is asked for', () => {
    expect(inviteAcceptsResponse({ method: 'REQUEST', cancelled: 0 })).toBe(true);
    expect(inviteAcceptsResponse({ method: null, cancelled: 0 })).toBe(true);
    expect(inviteAcceptsResponse({ method: 'PUBLISH', cancelled: 0 })).toBe(false);
    expect(inviteAcceptsResponse({ method: 'REPLY', cancelled: 0 })).toBe(false);
    expect(inviteAcceptsResponse({ method: 'REQUEST', cancelled: 1 })).toBe(false);
  });
});
