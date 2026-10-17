// @vitest-environment jsdom

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import NavItem from './NavItem.vue';

// jsdom hands out an http:// import.meta.url, so the file comes from the root.
const source = readFileSync(join(process.cwd(), 'src/components/ui/NavItem.vue'), 'utf8');

describe('NavItem', () => {
  it('says nothing about being pressed while it is plain navigation', () => {
    const wrapper = mount(NavItem, { props: { label: 'Posteingang', active: true } });

    expect(wrapper.attributes('aria-pressed')).toBeUndefined();
    expect(wrapper.classes()).toContain('nav-item--active');
  });

  it('announces the state once it is used as a toggle', () => {
    const on = mount(NavItem, { props: { label: 'Arbeit', active: true, pressed: true } });
    const off = mount(NavItem, { props: { label: 'Arbeit', pressed: false } });

    expect(on.attributes('aria-pressed')).toBe('true');
    expect(off.attributes('aria-pressed')).toBe('false');
  });

  /*
   * The settings navigation used to rebuild the row for this: the kit stood at
   * 34px, and a call site that adopted it lost its touch target on the phone
   * (UI-20). Same breakpoint that lifts the type scale in tokens.css.
   */
  it('grows to a touch target below the single-pane breakpoint', () => {
    const mobile = /@media \(max-width: 1020px\) \{([\s\S]*?)\n\}/.exec(source)?.[1] ?? '';

    expect(mobile).toMatch(/\.nav-item\s*\{/);
    expect(mobile).toMatch(/min-height:\s*48px/);
  });
});
