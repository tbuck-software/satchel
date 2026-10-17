import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/*
 * `@font-face` descriptors do not take custom properties. A `var()` in
 * `font-weight` voids the descriptor, the face registers as 400 only, and
 * every bold run in the UI is synthesised from the regular cut.
 */

const root = process.cwd();
const fontsCss = readFileSync(join(root, 'src/styles/fonts.css'), 'utf8');
const tokens = readFileSync(join(root, 'src/styles/tokens.css'), 'utf8');
const scale = new Set([...tokens.matchAll(/--weight-[\w-]+:\s*(\d+);/g)].map((match) => match[1]));

const faces = [...fontsCss.matchAll(/@font-face\s*\{([^}]*)\}/g)].map((match) => {
  const body = match[1] ?? '';
  return {
    family: /font-family:\s*'([^']+)'/.exec(body)?.[1] ?? '',
    weight: /font-weight:\s*([^;]+);/.exec(body)?.[1]?.trim() ?? '',
    body,
  };
});

describe('vendored font faces', () => {
  it('has faces to check', () => {
    expect(faces.length).toBeGreaterThan(0);
  });

  it('uses no custom properties in a descriptor', () => {
    expect(faces.filter((face) => face.body.includes('var(')).map((face) => face.family)).toEqual([]);
  });

  it('names weights from the scale in tokens.css', () => {
    for (const face of faces) {
      const weights = face.weight.split(/\s+/);
      expect(weights.every((weight) => scale.has(weight)), `${face.family}: ${face.weight}`).toBe(true);
    }
  });

  it('registers Figtree as the full variable range', () => {
    const figtree = faces.filter((face) => face.family === 'Figtree');
    expect(figtree.length).toBeGreaterThan(0);
    for (const face of figtree) expect(face.weight).toBe('400 700');
  });
});
