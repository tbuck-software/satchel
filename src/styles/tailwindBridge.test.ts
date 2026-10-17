import { readdirSync, readFileSync } from 'node:fs';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * `@theme inline` used to carry the colours and stop there. `text-sm` therefore
 * answered with Tailwind's 14px while `var(--font-size-sm)` answered with 13px,
 * and — the part a user could see — the phone-sized lift under 1020px never
 * reached a utility class, because it only moves the custom properties. Every
 * rung of both scales has to be bridged, so the next rung added to tokens.css
 * cannot quietly fall out again.
 */

const SOURCE_ROOT = fileURLToPath(new URL('..', import.meta.url));
const file = (name: string) => readFileSync(join(SOURCE_ROOT, 'styles', name), 'utf8');

const tokensCss = file('tokens.css');
const tailwindCss = file('tailwind.css');

function declarations(block: string): Map<string, string> {
  const found = new Map<string, string>();
  for (const [, name, value] of block.matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
    found.set(String(name), String(value).trim());
  }
  return found;
}

const rootTokens = declarations(/:root\s*\{([\s\S]*?)\n\}/.exec(tokensCss)?.[1] ?? '');
// The phone-sized scale, lifted at the breakpoint that splits list from detail.
const phoneTokens = declarations(
  /@media \(max-width: 1020px\)\s*\{\s*:root\s*\{([\s\S]*?)\n {2}\}/.exec(tokensCss)?.[1] ?? '',
);
const bridge = declarations(/@theme inline\s*\{([\s\S]*)\n\}/.exec(tailwindCss)?.[1] ?? '');

const TYPE_STEPS = ['3xs', '2xs', 'xs', 'sm', 'md', 'lg', 'xl', '2xl'];
// Tailwind multiplies one number; these are the rungs the token scale names.
const SPACE_STEPS: Record<string, number> = {
  '2xs': 0.5, xs: 1, '1-5': 1.5, sm: 2, '2-5': 2.5, md: 3, '3-5': 3.5, lg: 4, '5': 5, xl: 6, '2xl': 8,
};

/** The px value a bridged utility class resolves to, desktop or phone. */
function resolve(utility: string, width: 'desktop' | 'phone'): string {
  const bridged = bridge.get(`--text-${utility}`);
  expect(bridged, `text-${utility} is not bridged`).toBeDefined();
  const token = /^var\((--font-size-[\w-]+)\)$/.exec(String(bridged))?.[1];
  expect(token, `text-${utility} does not point at the scale: ${bridged}`).toBeDefined();
  const value = width === 'phone'
    ? phoneTokens.get(String(token)) ?? rootTokens.get(String(token))
    : rootTokens.get(String(token));
  return String(value);
}

function templates(): string[] {
  const walk = (root: string): string[] => readdirSync(root, { withFileTypes: true })
    .flatMap((entry) => {
      const path = join(root, entry.name);
      if (entry.isDirectory()) return walk(path);
      return entry.isFile() && extname(entry.name) === '.vue' ? [path] : [];
    });
  return walk(SOURCE_ROOT).map((path) => readFileSync(path, 'utf8'));
}

describe('tailwind bridge: the type scale', () => {
  it('reaches every step tokens.css defines', () => {
    const missing = TYPE_STEPS.filter(
      (step) => bridge.get(`--text-${step}`) !== `var(--font-size-${step})`,
    );

    expect(missing, 'add the step to @theme inline in tailwind.css').toEqual([]);
    // Nothing on the scale may be missing from the list this test walks.
    expect([...rootTokens.keys()].filter((name) => name.startsWith('--font-size-')))
      .toEqual(TYPE_STEPS.map((step) => `--font-size-${step}`));
  });

  it('makes text-sm and var(--font-size-sm) one value', () => {
    expect(resolve('sm', 'desktop')).toBe(rootTokens.get('--font-size-sm'));
    expect(resolve('sm', 'desktop')).toBe('13px');
  });

  it('drops the line heights Tailwind ships for its own steps', () => {
    // Only five of the eight rungs have one, so keeping them would make the
    // scale behave differently depending on which rung you stand on.
    for (const step of ['xs', 'sm', 'lg', 'xl', '2xl']) {
      expect(bridge.get(`--text-${step}--line-height`)).toBe('initial');
    }
  });

  /*
   * The sender line and the timestamp of a thread row: the densest text in the
   * app, and the reason this bridge exists. Both used to be nailed to a pixel
   * value and stayed desk-sized on a phone.
   */
  it('lifts a thread row with the media query', () => {
    expect(resolve('md', 'desktop')).toBe('14px');
    expect(resolve('md', 'phone')).toBe('16px');
    expect(resolve('xs', 'desktop')).toBe('12px');
    expect(resolve('xs', 'phone')).toBe('13px');
  });

  it('leaves no arbitrary type size behind to skip the lift', () => {
    const arbitrary = templates().flatMap((source) => [...source.matchAll(/text-\[\d+px\]/g)])
      .map((match) => match[0]);

    expect(arbitrary).toEqual([]);
  });
});

describe('tailwind bridge: the spacing scale', () => {
  it('pins Tailwind to the same unit the tokens use', () => {
    expect(bridge.get('--spacing')).toBe('4px');
  });

  it('lines gap-3 up with --space-md, and every other rung with its own', () => {
    const step = Number.parseFloat(String(bridge.get('--spacing')));
    const drift = Object.entries(SPACE_STEPS)
      .filter(([name, factor]) => rootTokens.get(`--space-${name}`) !== `${step * factor}px`)
      .map(([name]) => name);

    expect(drift, 'document the mapping in tailwind.css or fix the token').toEqual([]);
    expect(rootTokens.get('--space-md')).toBe(`${step * 3}px`);
  });
});
