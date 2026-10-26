import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * WebKit drew a blue ring around the whole shortcut help, the confirm dialogs
 * and the image lightbox whenever a key had opened them: the frame takes focus
 * from script so its name is announced, and WebKit counts that as keyboard
 * focus. One rule in base.css takes the ring off those frames — and only off
 * them, since everything else relies on the ring to show where the user is.
 */
const baseCss = readFileSync(
  join(fileURLToPath(new URL('.', import.meta.url)), 'base.css'),
  'utf8',
);

const outlineOffRules = [...baseCss.matchAll(/([^{}]+)\{[^{}]*\boutline\s*:\s*(?:none|0)\b[^{}]*\}/g)]
  .map((match) => match[1]!.replace(/\/\*[\s\S]*?\*\//g, '').trim());

describe('focus on dialog frames', () => {
  it('draws no ring around a dialog that only script can focus', () => {
    expect(outlineOffRules).toContain(
      ":is([role='dialog'], [role='alertdialog'])[tabindex='-1']:focus",
    );
  });

  it('switches the ring off nowhere else', () => {
    expect(outlineOffRules).toHaveLength(1);
  });
});
