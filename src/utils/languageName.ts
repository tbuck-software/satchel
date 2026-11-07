/**
 * A language code (`DE`, `EN-GB`, `ZH-HANS`) as the UI language names it:
 * "Deutsch", "British English". Falls back to the code when the runtime does
 * not know it, so a new engine language never shows up blank.
 */
export function languageName(code: string, locale: string): string {
  const tag = code.trim();
  if (!tag) return '';
  try {
    return new Intl.DisplayNames([locale], { type: 'language', fallback: 'code' }).of(tag) ?? tag;
  } catch {
    return tag;
  }
}
