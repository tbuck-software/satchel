import type { BackendCalendarInvite } from '@/types/calendar';

/**
 * iTIP methods that answer an invitation instead of extending one (RFC 5546):
 * an attendee's REPLY, COUNTER or REFRESH, the organizer's DECLINECOUNTER.
 * Mirrors `NON_INVITATION_METHODS` in `crates/satchel-core/src/email/calendar.rs`;
 * the backend no longer stores or lists them, the mock follows the same rule.
 */
const NON_INVITATION_METHODS = ['REPLY', 'COUNTER', 'REFRESH', 'DECLINECOUNTER'];

function normalizedMethod(invite: Pick<BackendCalendarInvite, 'method'>): string {
  return (invite.method ?? '').trim().toUpperCase();
}

/** Whether a calendar message is an invitation at all. No method counts as one. */
export function isInvitation(invite: Pick<BackendCalendarInvite, 'method'>): boolean {
  return !NON_INVITATION_METHODS.includes(normalizedMethod(invite));
}

/**
 * Whether Accept/Maybe/Decline make sense: an invitation that asks for an
 * answer. A PUBLISH announces an event and takes none; a cancelled meeting
 * takes none any more.
 */
export function inviteAcceptsResponse(
  invite: Pick<BackendCalendarInvite, 'method' | 'cancelled'>,
): boolean {
  return isInvitation(invite) && normalizedMethod(invite) !== 'PUBLISH' && (invite.cancelled ?? 0) !== 1;
}
