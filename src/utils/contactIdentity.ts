import { isValidEmailAddress } from './recipients';

// Inline attachment names can look like addresses, for example logo@2x.png.
const IMAGE_DOMAIN = /\.(?:png|jpe?g|gif|webp|svg|ico|bmp|tiff?)$/i;

export function isContactEmail(value: string): boolean {
  const email = value.trim();
  if (!isValidEmailAddress(email) || /[\\:]/u.test(email)) return false;
  for (const character of email) {
    if (character.charCodeAt(0) < 32) return false;
  }
  return !IMAGE_DOMAIN.test(email.slice(email.indexOf('@') + 1));
}

/** Decode quoted mail phrases, including legacy names with escaped outer quotes. */
export function cleanContactName(value?: string | null): string {
  let name = value?.trim().replace(/\\+"/g, '"') ?? '';
  if (name.startsWith('"')) {
    name = name.replace(/^"+/, '').replace(/"+(?=\s*(?:\(|$))/, '').trim();
  }
  // A service suffix without the sender's name is not a contact name.
  return /^\((?:(?:via|über) )?Google (?:Docs|Drive|Keep)\)$/i.test(name) ? '' : name;
}

export function contactLabel(name: string | null | undefined, email: string): string {
  const cleaned = cleanContactName(name);
  return cleaned && cleaned.toLowerCase() !== email.toLowerCase() ? cleaned : contactFallbackName(email);
}

export function contactFallbackName(email: string): string {
  // Calendar addresses use an opaque local part; the domain identifies the service.
  const [local = '', domain = ''] = email.split('@');
  return /^[a-f\d]{24,}$/i.test(local) ? domain : local;
}
