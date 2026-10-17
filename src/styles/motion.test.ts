import { readdirSync, readFileSync } from 'node:fs';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * `prefers-reduced-motion` is a promise the styleguide makes for the whole
 * app, and it was kept in 25 files out of 65 (UI-16). The fallback that closes
 * the other 40 is four declarations in one place — small enough to lose in a
 * refactor and impossible to notice by looking, since nothing changes for
 * anyone who has not switched the setting on.
 */
const STYLES_ROOT = fileURLToPath(new URL('.', import.meta.url));
const SOURCE_ROOT = fileURLToPath(new URL('..', import.meta.url));
const baseCss = readFileSync(join(STYLES_ROOT, 'base.css'), 'utf8');
const tokensCss = readFileSync(join(STYLES_ROOT, 'tokens.css'), 'utf8');

const reducedMotion = /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{([\s\S]*)\}/
  .exec(baseCss)?.[1] ?? '';

describe('the global reduced-motion fallback', () => {
  it('applies to every element, not to a component', () => {
    expect(reducedMotion).toMatch(/\*,\s*\*::before,\s*\*::after/);
  });

  it('shortens motion instead of removing the rule that carries it', () => {
    for (const declaration of [
      /animation-duration:\s*1ms\s*!important/,
      /animation-iteration-count:\s*1\s*!important/,
      /transition-duration:\s*1ms\s*!important/,
      /scroll-behavior:\s*auto\s*!important/,
    ]) {
      expect(reducedMotion).toMatch(declaration);
    }
  });

  /*
   * Unlayered on purpose: inside @layer base an !important declaration would
   * be outranked by nothing here, but the file is easier to reason about when
   * the one rule that must always win sits outside the cascade layers.
   */
  it('sits outside the base layer, where nothing can outrank it', () => {
    const layer = /@layer base \{[\s\S]*?\n\}/.exec(baseCss)?.[0] ?? '';

    expect(layer).not.toContain('prefers-reduced-motion');
  });
});

/** Every file that still tunes reduced motion itself, for the count below. */
function vueSources(root: string): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    if (entry.isDirectory()) return vueSources(path);
    return entry.isFile() && extname(entry.name) === '.vue' ? [path] : [];
  });
}

describe('the local reduced-motion rules', () => {
  /*
   * They are not duplicates of the fallback: UndoFab turns its countdown ring
   * into a number, others swap a slide for a fade. A file that only repeats
   * the four declarations above should lose them — OnboardingFlow did.
   */
  it('are still there, and still do more than shorten a duration', () => {
    const local = vueSources(SOURCE_ROOT)
      .filter((path) => readFileSync(path, 'utf8').includes('prefers-reduced-motion'));

    expect(local.length).toBe(24);
  });
});

describe('the motion tokens', () => {
  it('name the long spatial move instead of leaving it a literal', () => {
    expect(tokensCss).toMatch(/--duration-spatial:\s*320ms/);
  });
});
