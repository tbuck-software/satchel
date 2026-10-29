/**
 * Address-list parsing for the composer's recipient fields.
 *
 * A separator only separates outside a quoted display name and outside angle
 * brackets — `"Morgan, Alex" <t@example.com>` is one recipient, not two. The
 * chips, the paste handler and the send guard all read the list through here so
 * a name with a comma cannot be cut into an address that reaches nobody.
 */

const SEPARATOR = /[,;\n\r]/;

/** Deliberately narrow: no spaces, exactly one `@`, and a domain that looks like one. */
const ADDRESS_SHAPE = /^[^\s@,;<>"]+@[^\s@,;<>"]+$/;

interface ScanState {
  quoted: boolean;
  depth: number;
}

function scan(text: string, onSeparator?: (index: number) => void): ScanState {
  const state: ScanState = { quoted: false, depth: 0 };
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index];
    if (state.quoted && character === '\\') {
      index += 1;
      continue;
    }
    if (character === '"') {
      state.quoted = !state.quoted;
      continue;
    }
    if (state.quoted) continue;
    if (character === '<') state.depth += 1;
    else if (character === '>') state.depth = Math.max(0, state.depth - 1);
    else if (state.depth === 0 && SEPARATOR.test(character as string)) onSeparator?.(index);
  }
  return state;
}

/** Split an address list into recipient tokens, keeping display names intact. */
export function splitRecipientList(text: string): string[] {
  const tokens: string[] = [];
  let start = 0;
  scan(text, (index) => {
    tokens.push(text.slice(start, index));
    start = index + 1;
  });
  tokens.push(text.slice(start));
  return tokens.map((token) => token.trim()).filter(Boolean);
}

/**
 * True while the text ends inside a quoted name or an unclosed `<`. Typing a
 * comma there is part of the name, not the end of a recipient.
 */
export function hasOpenRecipientToken(text: string): boolean {
  const state = scan(text);
  return state.quoted || state.depth > 0;
}

export function isValidEmailAddress(value: string): boolean {
  const address = value.trim();
  if (!ADDRESS_SHAPE.test(address)) return false;
  const domain = address.slice(address.indexOf('@') + 1);
  return domain.includes('.') || domain.includes('-');
}

/** The bare address of a `Name <addr>` token, or null when it holds none. */
export function recipientAddress(token: string): string | null {
  const trimmed = token.trim();
  const open = trimmed.lastIndexOf('<');
  const close = trimmed.indexOf('>', open + 1);
  const candidate = open >= 0 && close > open ? trimmed.slice(open + 1, close).trim() : trimmed;
  return isValidEmailAddress(candidate) ? candidate : null;
}

/**
 * What a recipient chip shows: the display name of a `Name <addr>` token,
 * unquoted, or the token itself. The full token stays in the chip's title.
 */
export function recipientChipLabel(token: string): string {
  const trimmed = token.trim();
  const open = trimmed.lastIndexOf('<');
  if (open <= 0 || recipientAddress(trimmed) === null) return trimmed;
  const name = trimmed.slice(0, open).trim().replace(/^"(.*)"$/, '$1').trim();
  return name || trimmed;
}

export function isValidRecipient(token: string): boolean {
  return recipientAddress(token) !== null;
}

export function hasOnlyValidRecipients(tokens: Iterable<string>): boolean {
  for (const token of tokens) {
    if (!isValidRecipient(token)) return false;
  }
  return true;
}
