// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import MenuItem from './MenuItem.vue';

/**
 * In a menu the pointer and the keyboard are in two different places at once.
 * Hover and focus used to share one rule, so a keyboard user could not tell
 * which row they were on while the mouse rested on another (UI-09). The ring
 * is what separates them, and a screenshot is too coarse to prove a 2px ring,
 * so the assertion is made against the rules themselves.
 */
// jsdom gives import.meta.url a non-file scheme; the tests run from the repo root.
const source = readFileSync(join(process.cwd(), 'src/components/ui/MenuItem.vue'), 'utf8');
const styles = source.replace(/\/\*[\s\S]*?\*\//g, '');

/** The declarations of every rule whose selector list contains `selector`. */
function declarationsFor(selector: string): string[] {
  const declarations: string[] = [];
  for (const [, head, body] of styles.matchAll(/([^{}]*)\{([^{}]*)\}/g)) {
    const selectors = (head ?? '').split(',').map((entry) => entry.trim());
    if (!selectors.includes(selector)) continue;
    for (const declaration of (body ?? '').split(';')) {
      if (declaration.trim()) declarations.push(declaration.trim());
    }
  }
  return declarations;
}

describe('menu item', () => {
  it('draws keyboard focus differently from hover', () => {
    const hover = declarationsFor('.menu-item:hover:not(:disabled)');
    const focus = declarationsFor('.menu-item:focus-visible');

    expect(hover.length).toBeGreaterThan(0);
    expect(focus.filter((declaration) => !hover.includes(declaration))).not.toEqual([]);
  });

  it('takes the ring from the tokens rather than inventing one', () => {
    expect(declarationsFor('.menu-item:focus-visible')).toEqual(
      expect.arrayContaining([
        'outline: var(--border-focus)',
        'outline-offset: var(--focus-outline-offset)',
      ]),
    );
  });

  it('never switches the ring off', () => {
    expect(source).not.toMatch(/outline\s*:\s*(?:none|0)\b/);
  });
});

describe('menu item slots and tone', () => {
  it('puts a shortcut in the trailing slot, after the label', () => {
    const wrapper = mount(MenuItem, {
      props: { label: 'Reply' },
      slots: { trailing: '<kbd>r</kbd>' },
    });

    const children = wrapper.get('button').element.children;
    expect(wrapper.get('.menu-item__trailing kbd').text()).toBe('r');
    expect(children[children.length - 1]?.classList.contains('menu-item__trailing')).toBe(true);
  });

  it('draws a caller glyph through the icon slot', () => {
    const wrapper = mount(MenuItem, {
      props: { label: 'Export' },
      slots: { icon: '<span class="format">EML</span>' },
    });

    expect(wrapper.get('.menu-item__icon .format').text()).toBe('EML');
  });

  it('marks a destructive row with tone="danger"', () => {
    const danger = mount(MenuItem, { props: { label: 'Spam', tone: 'danger' } });
    const plain = mount(MenuItem, { props: { label: 'Read' } });

    expect(danger.get('button').classes()).toContain('menu-item--danger');
    expect(plain.get('button').classes()).not.toContain('menu-item--danger');
  });

  it('keeps the parent of an open flyout highlighted', () => {
    const wrapper = mount(MenuItem, { props: { label: 'Move', submenu: true, expanded: true } });

    expect(wrapper.get('button').attributes('aria-expanded')).toBe('true');
    expect(wrapper.get('button').classes()).toContain('menu-item--expanded');
  });

  it('mixes the danger hover with transparency, not with black', () => {
    expect(styles).not.toMatch(/--danger[^}]*black/);
    expect(declarationsFor('.menu-item--danger:hover').join(';')).toContain('transparent');
  });
});
