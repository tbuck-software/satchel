import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { ACCOUNT_COLORS, isAccountColor, wantsDarkInk } from './accountColors';

describe('account colours', () => {
  it('matches the palette new accounts take their colour from', () => {
    const rust = readFileSync('crates/satchel-core/src/db/accounts.rs', 'utf8');
    const block = /pub const ACCOUNT_COLORS: \[&str; \d+\] = \[([^\]]*)\]/.exec(rust)?.[1] ?? '';
    const defaultColor = /const DEFAULT_COLOR: &str = "([^"]+)"/.exec(rust)?.[1];
    const values = block.split(',').map((entry) => entry.trim()).filter(Boolean)
      .map((entry) => (entry === 'DEFAULT_COLOR' ? defaultColor : entry.replaceAll('"', '')));
    expect(values).toEqual(ACCOUNT_COLORS.map((color) => color.value));
  });

  it('accepts only #rrggbb', () => {
    expect(isAccountColor('#A97FFF')).toBe(true);
    expect(isAccountColor('blue')).toBe(false);
    expect(isAccountColor('#abc')).toBe(false);
  });

  it('puts dark initials on light fills and light initials on dark ones', () => {
    expect(wantsDarkInk('#f0b232')).toBe(true);
    expect(wantsDarkInk('#4cd68a')).toBe(true);
    expect(wantsDarkInk('#1d3a6b')).toBe(false);
    expect(wantsDarkInk('#a97fff')).toBe(true);
  });
});
