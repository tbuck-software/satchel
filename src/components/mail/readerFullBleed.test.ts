import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/*
 * On the phone a designed HTML mail runs edge to edge, while simple mail,
 * headers, attachments and notices keep the reader's side inset. Four files
 * take part, and each one alone breaks the rule without a failing render:
 *
 * - ThreadView names the inset as --reader-inline-inset on the phone scroller;
 * - MessageBubble widens its own box by it (paint containment would clip a
 *   child reaching past a narrower one) and pads its parts back in;
 * - EmailRenderer takes the document shell out by the same amount;
 * - IsolatedEmailDocument's flush rules pad every body without a fixed-width
 *   layout table by the inset itself, so its text lines up with the header
 *   and only designed layouts touch the edges.
 */
const read = (path: string) => readFileSync(fileURLToPath(new URL(path, import.meta.url)), 'utf8');

function px(token: string, css: string): number {
  const match = new RegExp(`${token}:\\s*(\\d+)px`).exec(css);
  if (!match) throw new Error(`${token} not found`);
  return Number(match[1]);
}

describe('designed mail runs edge to edge on the phone', () => {
  const threadView = read('./ThreadView.vue');
  const bubble = read('./MessageBubble.vue');
  const renderer = read('./EmailRenderer.vue');
  const isolated = read('./IsolatedEmailDocument.vue');
  const tokens = read('../../styles/tokens.css');

  it('names the phone reader inset where the scroller pads by it', () => {
    const phone = /\.thread-reading-content--phone \{([^}]*)\}/.exec(threadView)?.[1] ?? '';
    expect(phone).toContain('--reader-inline-inset: var(--space-lg)');
    expect(phone).toMatch(/padding:[^;]*var\(--reader-inline-inset\)/);
    // The utility padding that used to set the inset would be a second copy.
    expect(threadView).not.toContain("'thread-reading-content--phone px-4");
  });

  it('widens the phone message by the inset and pads its parts back', () => {
    expect(bubble).toMatch(/\.message-surface--phone \{\s*margin-inline: calc\(var\(--reader-inline-inset, 0px\) \* -1\);/);
    expect(bubble).toMatch(/\.message-surface--phone > \* \{\s*padding-inline: var\(--reader-inline-inset, 0px\);/);
    expect(bubble).toContain("'message-surface--phone ");
  });

  it('takes the document shell out of the inset at phone width only', () => {
    const phoneBlocks = [...renderer.matchAll(/@media \(max-width: 767px\) \{([\s\S]*?)\n\}/g)].map((m) => m[1] ?? '');
    expect(phoneBlocks.some((block) => /\.html-document-shell \{\s*margin-inline: calc\(var\(--reader-inline-inset, 0px\) \* -1\);/.test(block))).toBe(true);
    // Only the frame's shell bleeds: the image notice, the switch and plain
    // text are siblings of it and keep the inset.
    expect(renderer).not.toMatch(/\.(renderer|image-guard|plain-document-group|html-document-group) \{[^}]*margin-inline/);
  });

  it('pads all but fixed-width layouts back to the inset inside the flush frame', () => {
    const inset = px('--space-lg', tokens);
    const flush = /props\.flush \? `([\s\S]*?)` : ''/.exec(isolated)?.[1] ?? '';
    const simple = /\.satchel-email-body:not\(:has\(table\[width\][^{]*\{\s*padding: (\d+)px (\d+)px (\d+)px/.exec(flush);
    expect(simple).not.toBeNull();
    expect(Number(simple![2])).toBe(inset);
  });
});
