/**
 * Webmail start pages of well-known providers, by mail domain.
 *
 * Out of office can only be set by Satchel for Gmail and Outlook; everyone
 * else sets it in their provider's webmail. This list turns that advice into
 * a link. It is deliberately short and best effort: a domain that is missing
 * simply gets no link. To add a provider, add its mail domains with the page
 * its users sign in to — one entry per domain, https only.
 */
const WEBMAIL_BY_DOMAIN: Readonly<Record<string, string>> = {
  'gmx.de': 'https://www.gmx.net/',
  'gmx.net': 'https://www.gmx.net/',
  'gmx.at': 'https://www.gmx.at/',
  'gmx.ch': 'https://www.gmx.ch/',
  'web.de': 'https://web.de/',
  't-online.de': 'https://email.t-online.de/',
  'posteo.de': 'https://posteo.de/',
  'posteo.net': 'https://posteo.de/',
  'mailbox.org': 'https://login.mailbox.org/',
  'mail.de': 'https://mail.de/',
  'ionos.de': 'https://mail.ionos.de/',
  'icloud.com': 'https://www.icloud.com/mail',
  'me.com': 'https://www.icloud.com/mail',
  'mac.com': 'https://www.icloud.com/mail',
  'yahoo.com': 'https://mail.yahoo.com/',
  'yahoo.de': 'https://mail.yahoo.com/',
  'aol.com': 'https://mail.aol.com/',
  'fastmail.com': 'https://app.fastmail.com/',
  'proton.me': 'https://mail.proton.me/',
  'protonmail.com': 'https://mail.proton.me/',
  'zoho.com': 'https://mail.zoho.com/',
};

/** The webmail page for an address, or null when its provider is not listed. */
export function webmailFor(email: string): string | null {
  const domain = email.split('@').pop()?.trim().toLowerCase() ?? '';
  return WEBMAIL_BY_DOMAIN[domain] ?? null;
}
