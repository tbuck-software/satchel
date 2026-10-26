import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * The five light-theme breakages all had the same shape: a literal colour in a
 * component that only ever worked against the dark palette. This resolves the
 * real tokens for both themes and measures the four surfaces that were fixed.
 *
 * Deliberately narrow. The repo-wide design-system check is a separate concern;
 * this file exists so those four places cannot silently regress.
 */

const src = (path: string) => readFileSync(fileURLToPath(new URL(path, import.meta.url)), 'utf8');

const tokensCss = src('./tokens.css');

function tokensFor(theme: 'dark' | 'light'): Map<string, string> {
  // The dark palette lives in :root; the light theme overrides a subset.
  const blocks = [/:root\s*\{([\s\S]*?)\n\}/.exec(tokensCss)?.[1] ?? ''];
  if (theme === 'light') {
    blocks.push(/\[data-theme='light'\]\s*\{([\s\S]*?)\n\}/.exec(tokensCss)?.[1] ?? '');
  }
  const tokens = new Map<string, string>();
  for (const block of blocks) {
    for (const match of block.matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
      tokens.set(String(match[1]), String(match[2]).trim());
    }
  }
  return tokens;
}

const THEMES = { dark: tokensFor('dark'), light: tokensFor('light') } as const;

function rgb(token: string, theme: keyof typeof THEMES): [number, number, number] {
  const value = THEMES[theme].get(token);
  if (!value) throw new Error(`unknown token ${token} in ${theme}`);
  const match = /^#([0-9a-f]{6})$/i.exec(value);
  if (!match?.[1]) throw new Error(`${token} is not a plain hex colour in ${theme}: ${value}`);
  const n = parseInt(match[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

/** WCAG 2.1 relative luminance and contrast ratio. */
function luminance([r, g, b]: [number, number, number]): number {
  const channel = (v: number) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a: string, b: string, theme: keyof typeof THEMES): number {
  const first = luminance(rgb(a, theme));
  const second = luminance(rgb(b, theme));
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

const THEME_NAMES = ['dark', 'light'] as const;

type Rgb = [number, number, number];

/**
 * `rgb()` above wants a plain hex, which the surface tokens are. The hover
 * fills are derived, so they need the two steps in between: a `var()` and one
 * `color-mix(in srgb, <colour> <n>%, <colour>)`.
 */
function resolve(value: string, theme: keyof typeof THEMES, depth = 0): Rgb {
  const text = value.trim();
  if (depth > 6) throw new Error(`cannot resolve ${value}`);
  if (text === 'white') return [255, 255, 255];
  if (text === 'black') return [0, 0, 0];

  const hex = /^#([0-9a-f]{6})$/i.exec(text);
  if (hex?.[1]) {
    const n = parseInt(hex[1], 16);
    return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
  }

  const variable = /^var\(\s*(--[\w-]+)\s*\)$/.exec(text);
  if (variable?.[1]) {
    const declared = THEMES[theme].get(variable[1]);
    if (!declared) throw new Error(`unknown token ${variable[1]} in ${theme}`);
    return resolve(declared, theme, depth + 1);
  }

  const mix = /^color-mix\(\s*in\s+srgb\s*,\s*(.+?)\s+([\d.]+)%\s*,\s*(.+)\)$/i.exec(text);
  if (mix?.[1] && mix[2] && mix[3]) {
    const weight = parseFloat(mix[2]) / 100;
    const first = resolve(mix[1], theme, depth + 1);
    const second = resolve(mix[3], theme, depth + 1);
    return [0, 1, 2].map((i) => (first[i] as number) * weight
      + (second[i] as number) * (1 - weight)) as unknown as Rgb;
  }

  throw new Error(`cannot resolve ${value} in ${theme}`);
}

/** WCAG contrast of --text-on-accent (#ffffff) on a fill. */
function onAccent(fill: string, theme: keyof typeof THEMES): number {
  return 1.05 / (luminance(resolve(fill, theme)) + 0.05);
}

describe('FolderBrowserPanel (UI-01)', () => {
  const source = src('../components/layout/FolderBrowserPanel.vue');

  it('draws its surface from tokens', () => {
    expect(source).toContain('background: var(--bg-overlay);');
    expect(source).toContain('box-shadow: var(--shadow-lg);');
    expect(source).not.toMatch(/rgba\(|#[0-9a-f]{3,8}\b/i);
    expect(source).not.toMatch(/font-size:\s*\d/);
  });

  it.each(THEME_NAMES)('carries text on --bg-overlay in the %s theme', (theme) => {
    expect(contrast('--text-primary', '--bg-overlay', theme)).toBeGreaterThanOrEqual(4.5);
    expect(contrast('--text-secondary', '--bg-overlay', theme)).toBeGreaterThanOrEqual(4.5);
    expect(contrast('--text-tertiary', '--bg-overlay', theme)).toBeGreaterThanOrEqual(4.5);
  });
});

describe('AttachmentChip (UI-02)', () => {
  const source = src('../components/mail/AttachmentChip.vue');

  it('uses token classes, not the Tailwind palette', () => {
    expect(source).not.toMatch(/text-(slate|gray|zinc|neutral|stone)-/);
    expect(source).toContain('text-primary');
    expect(source).toContain('text-tertiary');
  });

  it.each(THEME_NAMES)('keeps filename and size readable on --bg-surface in %s', (theme) => {
    expect(contrast('--text-primary', '--bg-surface', theme)).toBeGreaterThanOrEqual(4.5);
    expect(contrast('--text-tertiary', '--bg-surface', theme)).toBeGreaterThanOrEqual(4.5);
  });
});

describe('account setup surfaces (UI-04)', () => {
  it('AccountSetup has no white-alpha fills left', () => {
    const source = src('../components/settings/AccountSetup.vue');
    expect(source).not.toMatch(/rgba\(/);
  });

  it('MailboxTree has no white-alpha utility classes left', () => {
    expect(src('../components/layout/MailboxTree.vue')).not.toContain('bg-white/[');
  });

  it.each(THEME_NAMES)('separates the cards from the settings page in %s', (theme) => {
    // The cards sit on --bg-base. The fill alone is only a ~1.1:1 step in both
    // themes, so the border is what has to carry — measure it.
    expect(contrast('--bg-surface', '--bg-base', theme)).toBeGreaterThan(1);
    expect(contrast('--border-color', '--bg-base', theme)).toBeGreaterThanOrEqual(1.2);
    expect(contrast('--bg-raised', '--bg-surface', theme)).toBeGreaterThan(1);
    expect(contrast('--text-secondary', '--bg-raised', theme)).toBeGreaterThanOrEqual(4.5);
  });
});

describe('accent hover fills (UI-15)', () => {
  /*
   * The bug this replaces: AppButton mixed its hover toward white in both
   * themes, so on a pale page the primary button went *paler* when the pointer
   * arrived — the feedback ran backwards. Direction is the whole promise, and
   * the resting and hovered fills must move in the intended direction.
   */
  it.each(THEME_NAMES)('keeps white text readable on primary fills in %s', (theme) => {
    expect(onAccent('var(--accent-primary)', theme)).toBeGreaterThanOrEqual(4.5);
    expect(onAccent('var(--accent-primary-hover)', theme)).toBeGreaterThanOrEqual(4.5);
  });

  it('brightens the primary fill in the dark theme and darkens it in the light one', () => {
    expect(luminance(resolve('var(--accent-primary-hover)', 'dark')))
      .toBeGreaterThan(luminance(resolve('var(--accent-primary)', 'dark')));
    expect(luminance(resolve('var(--accent-primary-hover)', 'light')))
      .toBeLessThan(luminance(resolve('var(--accent-primary)', 'light')));
  });

  /*
   * Danger does not turn with the theme. --text-on-accent is white in both, and
   * a lightened #f85149 carries it at 2.9:1. Darkening in both directions costs
   * the symmetry and buys the reading.
   */
  it.each(THEME_NAMES)('darkens the danger fill in the %s theme, for the white ink', (theme) => {
    expect(luminance(resolve('var(--accent-red-hover)', theme)))
      .toBeLessThan(luminance(resolve('var(--accent-red)', theme)));
    expect(onAccent('var(--accent-red-hover)', theme)).toBeGreaterThanOrEqual(4.5);
  });

  it('is what the kit uses, instead of a mix of its own', () => {
    const source = src('../components/ui/AppButton.vue');

    expect(source).toContain('background: var(--accent-primary-hover);');
    expect(source).not.toMatch(/color-mix\([^;]*\b(?:white|black)\b/);
  });
});


describe('preference and composer controls', () => {
  it.each(THEME_NAMES)('keeps off switches visible against cards in %s', (theme) => {
    for (const surface of ['--bg-raised', '--bg-surface']) {
      expect(contrast('--control-off', surface, theme)).toBeGreaterThanOrEqual(3);
      expect(contrast('--control-off-border', surface, theme)).toBeGreaterThanOrEqual(3);
    }
  });

  it.each(THEME_NAMES)('keeps inactive Send and placeholders readable in %s', (theme) => {
    expect(onAccent('color-mix(in srgb, var(--accent-primary) 75%, var(--bg-raised))', theme))
      .toBeGreaterThanOrEqual(3);
    expect(contrast('--text-secondary', '--bg-base', theme)).toBeGreaterThanOrEqual(4.5);
    expect(contrast('--text-secondary', '--bg-raised', theme)).toBeGreaterThanOrEqual(4.5);
  });
});
