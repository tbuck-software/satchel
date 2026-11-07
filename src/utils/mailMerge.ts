import type { BackendMergeList, BackendMergeRecipient } from '@/types/mailMerge';

/** The list a mail merge sends to, and where it came from. */
export interface MailMergeSource extends BackendMergeList {
  kind: 'csv' | 'contacts';
  /** File name or "12 contacts", shown in the composer and on the job. */
  label: string;
}

/** Field names a contact offers as placeholders. */
export const CONTACT_FIELDS = ['FirstName', 'LastName', 'Name', 'Email'] as const;

/**
 * First and last name from a display name. "Meier, Anna" is last-first;
 * otherwise the last word is the last name and the rest the first name.
 */
export function splitDisplayName(raw: string): { first: string; last: string } {
  const name = raw.trim().replace(/\s+/g, ' ');
  if (!name || name.includes('@')) return { first: '', last: '' };
  const comma = name.indexOf(',');
  if (comma > 0) {
    return { first: name.slice(comma + 1).trim(), last: name.slice(0, comma).trim() };
  }
  const words = name.split(' ');
  if (words.length === 1) return { first: words[0]!, last: '' };
  return { first: words.slice(0, -1).join(' '), last: words[words.length - 1]! };
}

/** A recipient list from contacts: one row each, the name split into fields. */
export function contactsMergeSource(
  contacts: ReadonlyArray<{ email: string; name: string }>,
  label: string,
): MailMergeSource {
  const rows: BackendMergeRecipient[] = contacts.map((contact) => {
    const name = contact.name.trim() === contact.email.trim() ? '' : contact.name.trim();
    const { first, last } = splitDisplayName(name);
    return {
      line: null,
      values: { FirstName: first, LastName: last, Name: name, Email: contact.email.trim() },
    };
  });
  return { kind: 'contacts', label, fields: [...CONTACT_FIELDS], emailField: 'Email', rows };
}

/** The token a field picker inserts. */
export function mergeToken(field: string): string {
  return `{{${field}}}`;
}

/** Whether a text carries a placeholder at all; a merge without one is a plain copy. */
export function hasMergeToken(text: string): boolean {
  return /\{\{[^{}]*[^\s{}|][^{}]*\}\}/.test(text);
}

/**
 * How long a job of `count` messages takes at `delaySeconds` apart, in whole
 * minutes, rounded up. The first message goes at once.
 */
export function mergeDurationMinutes(count: number, delaySeconds: number): number {
  if (count <= 1) return 0;
  return Math.ceil(((count - 1) * delaySeconds) / 60);
}

/** Reads a picked file as base64 for `parse_mail_merge_csv`. */
export async function fileToBase64(file: Blob): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = '';
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}
