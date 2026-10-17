// @vitest-environment jsdom

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import AppButton from './AppButton.vue';

/**
 * The settings panes each carried their own primary and their own outlined
 * twin, and the two never matched in height (UI-07). Both now come from here,
 * so the size classes — not the call site — decide the geometry.
 */
// jsdom hands out an http:// import.meta.url, so the file comes from the root.
const source = readFileSync(join(process.cwd(), 'src/components/ui/AppButton.vue'), 'utf8');
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

describe('AppButton', () => {
  it('carries the variant and the size as classes', () => {
    const wrapper = mount(AppButton, { props: { variant: 'secondary', size: 'sm' } });

    expect(wrapper.classes()).toEqual(
      expect.arrayContaining(['app-button', 'app-button--secondary', 'app-button--sm']),
    );
  });

  it('stays a soft medium button when nothing is asked for', () => {
    const wrapper = mount(AppButton);

    expect(wrapper.classes()).toEqual(
      expect.arrayContaining(['app-button--soft', 'app-button--md']),
    );
    expect(wrapper.attributes('type')).toBe('button');
  });

  /*
   * A form with more than one field loses its implicit Enter submission when
   * the submit button is not inside it. Two settings forms hand-rolled a pill
   * for exactly that (UI-20).
   */
  it('can be the submit button of a form', () => {
    const wrapper = mount(AppButton, { props: { type: 'submit' } });

    expect(wrapper.attributes('type')).toBe('submit');
  });

  it('does not fire while disabled', async () => {
    const wrapper = mount(AppButton, { props: { variant: 'primary', disabled: true } });

    await wrapper.trigger('click');

    expect(wrapper.attributes('disabled')).toBeDefined();
    expect(wrapper.emitted('click')).toBeUndefined();
  });

  it('fires otherwise', async () => {
    const wrapper = mount(AppButton, { props: { variant: 'secondary' } });

    await wrapper.trigger('click');

    expect(wrapper.emitted('click')).toHaveLength(1);
  });
});

describe('the secondary variant', () => {
  it('is an outline on a transparent ground', () => {
    expect(declarationsFor('.app-button--secondary')).toEqual([
      'background: transparent',
      'color: var(--text-primary)',
      'border: 1px solid var(--border-color)',
    ]);
  });

  it('answers the pointer, the way every other variant does', () => {
    expect(declarationsFor('.app-button--secondary:hover:not(:disabled)')).toEqual([
      'background: var(--bg-hover)',
    ]);
  });

  /*
   * The point of the variant: a primary and a secondary in one row line up
   * because both take their height from the size class. A variant that
   * declared its own padding or height would undo that.
   */
  it('leaves the geometry to the size classes', () => {
    for (const variant of ['primary', 'soft', 'secondary', 'ghost', 'danger']) {
      const own = declarationsFor(`.app-button--${variant}`).join(' ');

      expect(own).not.toMatch(/\b(?:height|padding|font-size)\s*:/);
    }
    expect(declarationsFor('.app-button--md')).toEqual(
      expect.arrayContaining(['height: 36px', 'padding: 0 var(--space-lg)', 'font-size: var(--font-size-sm)']),
    );
  });

  it('dims with the shared disabled rule instead of one of its own', () => {
    expect(declarationsFor('.app-button:disabled')).toEqual(
      expect.arrayContaining(['opacity: 0.45', 'cursor: default']),
    );
    expect(styles).not.toMatch(/\.app-button--\w+:disabled/);
  });
});
