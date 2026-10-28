/**
 * The colours an account can wear. They are data, not theme colours: the
 * value is stored on the account and has to read the same in both themes,
 * like the sender palette in `services/senderAvatar.ts`.
 *
 * `ACCOUNT_COLORS` in `crates/satchel-core/src/db/accounts.rs` is the same
 * list in the same order; a new account takes the first one nobody wears.
 */
export const ACCOUNT_COLORS = [
  { id: 'blue', value: '#55a6ff' },
  { id: 'green', value: '#4cd68a' },
  { id: 'amber', value: '#f0b232' },
  { id: 'purple', value: '#a97fff' },
  { id: 'cyan', value: '#3ad4e5' },
  { id: 'pink', value: '#f06898' },
  { id: 'orange', value: '#ff9f43' },
  { id: 'red', value: '#ff5a5a' },
] as const;

export type AccountColorId = (typeof ACCOUNT_COLORS)[number]['id'];

export const DEFAULT_ACCOUNT_COLOR = ACCOUNT_COLORS[0].value;

export function isAccountColor(value: string): boolean {
  return /^#[0-9a-f]{6}$/i.test(value);
}

/**
 * Whether initials on this fill want dark ink. Relative luminance after the
 * sRGB curve; above the point where white text loses its 4.5:1 contrast.
 */
export function wantsDarkInk(color: string): boolean {
  if (!isAccountColor(color)) return false;
  const channel = (offset: number) => {
    const value = Number.parseInt(color.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  const luminance = 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
  return luminance > 0.18;
}
