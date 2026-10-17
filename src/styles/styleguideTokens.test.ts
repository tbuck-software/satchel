import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/*
 * STYLEGUIDE.md documents the scales and tokens.css defines them. They drifted
 * once already — the guide knew seven type sizes while the tree used twenty —
 * so every documented step has to exist with its value, and every token of a
 * scale has to be documented (UI-11).
 */

const root = process.cwd();
const guide = readFileSync(join(root, 'STYLEGUIDE.md'), 'utf8');
const tokens = readFileSync(join(root, 'src/styles/tokens.css'), 'utf8');
const rootBlock = /:root\s*\{([\s\S]*?)\n\}/.exec(tokens)?.[1] ?? '';

function defined(prefix: string): Map<string, string> {
  const found = new Map<string, string>();
  for (const [, name, value] of rootBlock.matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
    if (String(name).startsWith(prefix)) found.set(String(name), String(value).trim());
  }
  return found;
}

/** `| `--token` | value | …` rows of the guide for one prefix. */
function documented(prefix: string): Map<string, string> {
  const found = new Map<string, string>();
  for (const [, name, value] of guide.matchAll(/^\|\s*`(--[\w-]+)`\s*\|\s*([^|]+?)\s*\|/gm)) {
    if (String(name).startsWith(prefix)) found.set(String(name), String(value).trim());
  }
  return found;
}

describe('STYLEGUIDE.md and tokens.css agree', () => {
  it.each(['--font-size-', '--weight-', '--space-', '--radius-'])('on %s', (prefix) => {
    const inGuide = documented(prefix);
    const inTokens = defined(prefix);

    expect(inGuide.size, `no ${prefix} table in STYLEGUIDE.md`).toBeGreaterThan(0);
    expect(Object.fromEntries(inGuide)).toEqual(Object.fromEntries(inTokens));
  });
});
