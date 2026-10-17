/** Domain part of an email address, lowercased. Empty string when there is none. */
export function senderDomain(email: string): string {
  const at = email.lastIndexOf('@');
  if (at === -1 || at === email.length - 1) return '';
  return email.slice(at + 1).trim().toLowerCase();
}

/** `strato.de`, `@strato.de` and `STRATO.DE` all mean the same domain. */
export function normalizeDomain(domain: string): string {
  return domain.trim().toLowerCase().replace(/^@+/, '');
}

/**
 * Providers where a shared domain says nothing about the sender: allowing all
 * of `gmail.com` would wave through every stranger with a Gmail address, so
 * these domains never get a domain-wide allow suggestion.
 */
const PERSONAL_MAILBOX_DOMAINS = new Set([
  'gmail.com',
  'googlemail.com',
  'outlook.com',
  'outlook.de',
  'hotmail.com',
  'hotmail.de',
  'live.com',
  'live.de',
  'msn.com',
  'yahoo.com',
  'yahoo.de',
  'ymail.com',
  'gmx.de',
  'gmx.net',
  'gmx.at',
  'gmx.ch',
  'web.de',
  'icloud.com',
  'me.com',
  'mac.com',
  'protonmail.com',
  'proton.me',
  'pm.me',
  't-online.de',
  'magenta.de',
  'freenet.de',
  'mail.de',
  'mailbox.org',
  'posteo.de',
  'posteo.net',
  'aol.com',
  'arcor.de',
  'vodafone.de',
]);

export function isPersonalMailboxDomain(domain: string): boolean {
  return PERSONAL_MAILBOX_DOMAINS.has(normalizeDomain(domain));
}
