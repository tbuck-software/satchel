import type { Account, Participant } from '@/types/mail';

type MailAccountIdentity = Pick<Account, 'displayName' | 'email' | 'senderName'>;

function normalizedEmail(email?: string | null): string {
  return email?.trim().toLowerCase() ?? '';
}

export function ownAccountForEmail(
  accounts: readonly MailAccountIdentity[],
  email?: string | null,
): MailAccountIdentity | undefined {
  const target = normalizedEmail(email);
  if (!target) return undefined;
  return accounts.find((account) => normalizedEmail(account.email) === target);
}

export function isOwnEmail(
  accounts: readonly MailAccountIdentity[],
  email?: string | null,
): boolean {
  return Boolean(ownAccountForEmail(accounts, email));
}

export function participantDisplayName(
  accounts: readonly MailAccountIdentity[],
  participant: Participant,
): string {
  const ownAccount = ownAccountForEmail(accounts, participant.email);
  // The person's own name, not the label they gave the account ("Work").
  const ownName = ownAccount ? (ownAccount.senderName ?? ownAccount.displayName).trim() : '';
  return ownName || participant.name.trim() || participant.email;
}
