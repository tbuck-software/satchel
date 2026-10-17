/**
 * Strip what is invisible but compares as different.
 *
 * Addresses and bodies pasted out of other clients carry zero-width joiners
 * and non-breaking spaces. Without this the dirty check reports changes the
 * user cannot see, and the composer asks to save a draft nobody edited.
 */
export function normalizeVisibleText(value: string): string {
  return value
    .replace(/[\u200B-\u200D\u2060\uFEFF]/g, '')
    .replace(/\u00A0/g, ' ')
    .replace(/\r\n?/g, '\n')
    .trim();
}

export function normalizedAddresses(values: readonly string[]): string[] {
  return values.map(normalizeVisibleText).filter(Boolean);
}
