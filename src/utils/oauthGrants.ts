/**
 * Whether an account's recorded OAuth grant covers an optional feature.
 *
 * Google reports exact scope URLs. Microsoft reports Graph permissions with
 * or without their resource prefix and in the registration's spelling, so
 * Outlook scopes compare by their bare, case-folded name — as the backend's
 * `outlook_scope_granted` does.
 */
const GOOGLE_SCOPES = {
  calendar: 'https://www.googleapis.com/auth/calendar.readonly',
  contacts: 'https://www.googleapis.com/auth/contacts.readonly',
} as const;

const OUTLOOK_CALENDAR = 'calendars.readwrite';

function bareScope(scope: string): string {
  return scope.slice(scope.lastIndexOf('/') + 1).toLowerCase();
}

export function hasOptionalGrant(
  provider: 'gmail' | 'outlook',
  purpose: 'contacts' | 'calendar',
  grantedScopes: readonly string[] | null | undefined,
): boolean {
  const scopes = grantedScopes ?? [];
  if (provider === 'outlook') {
    return purpose === 'calendar' && scopes.some((scope) => bareScope(scope) === OUTLOOK_CALENDAR);
  }
  return scopes.includes(GOOGLE_SCOPES[purpose]);
}
