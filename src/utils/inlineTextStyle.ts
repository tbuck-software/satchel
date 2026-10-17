/**
 * Inline text styling that survives the editor.
 *
 * The rich-text editor keeps only what its schema knows. Everything else is
 * dropped silently on load — which is how an adopted signature lost its brand
 * colour and font: the mail carried `<strong style="color:#48c725">`, the
 * editor parsed the bold and threw the rest away.
 *
 * These declarations are the ones worth carrying: they change how text reads,
 * not where it sits. Layout properties stay out on purpose — a signature that
 * brings its own margins, floats or positioning fights the composer instead of
 * living in it.
 */
const KEPT_DECLARATIONS = new Set([
  'color',
  'background-color',
  'font-family',
  'font-size',
  'font-style',
  'font-weight',
  'text-decoration',
  'text-transform',
]);

/** Values that pull in anything external or executable are never worth it. */
const REJECTED_VALUE = /url\(|expression\(|javascript:|@import/i;

/**
 * Reduces a style attribute to the text declarations above, in source order.
 * Returns null when nothing worth keeping is left, which lets a caller treat
 * "no styling" and "only layout styling" the same way.
 */
export function keepTextDeclarations(style: string | null | undefined): string | null {
  if (!style) return null;

  const kept: string[] = [];
  for (const declaration of style.split(';')) {
    const separator = declaration.indexOf(':');
    if (separator < 0) continue;

    const property = declaration.slice(0, separator).trim().toLowerCase();
    const value = declaration.slice(separator + 1).trim();
    if (!value || !KEPT_DECLARATIONS.has(property)) continue;
    if (REJECTED_VALUE.test(value)) continue;

    kept.push(`${property}: ${value}`);
  }

  return kept.length > 0 ? kept.join('; ') : null;
}

/** Reads one declaration back out, for showing what the caret sits in. */
export function declarationValue(
  style: string | null | undefined,
  property: string,
): string | null {
  const normalized = keepTextDeclarations(style);
  if (!normalized) return null;

  for (const declaration of normalized.split(';')) {
    const separator = declaration.indexOf(':');
    if (separator < 0) continue;
    if (declaration.slice(0, separator).trim() === property) {
      return declaration.slice(separator + 1).trim();
    }
  }
  return null;
}

/**
 * Sets or clears a single declaration, leaving the others as they were —
 * picking a colour must not drop the font the text already carries. A null
 * value removes the property; a result of null means nothing is left to keep.
 */
export function withDeclaration(
  style: string | null | undefined,
  property: string,
  value: string | null,
): string | null {
  const kept = (keepTextDeclarations(style) ?? '')
    .split(';')
    .map((declaration) => declaration.trim())
    .filter((declaration) => declaration && !declaration.startsWith(`${property}:`));

  if (value) kept.push(`${property}: ${value}`);
  return keepTextDeclarations(kept.join('; '));
}

/**
 * The legacy `<font color="…">` form, translated into the same shape. Mail
 * signatures written years ago still arrive this way.
 */
export function fontElementStyle(color: string | null, face: string | null): string | null {
  const declarations = [
    color?.trim() ? `color: ${color.trim()}` : null,
    face?.trim() ? `font-family: ${face.trim()}` : null,
  ].filter((declaration): declaration is string => declaration !== null);

  if (declarations.length === 0) return null;
  return keepTextDeclarations(declarations.join('; '));
}
