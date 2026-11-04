// @vitest-environment jsdom

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import TabBar from './TabBar.vue';

// jsdom hands out an http:// import.meta.url, so the file comes from the root.
const source = readFileSync(join(process.cwd(), 'src/components/ui/TabBar.vue'), 'utf8');

const items = [
  { id: 'mail', label: 'Mail' },
  { id: 'calendar', label: 'Calendar' },
  { id: 'settings', label: 'Settings' },
];

function mountBar(active: string | null = 'mail') {
  return mount(TabBar, {
    props: { label: 'Areas', items, active },
    slots: { icon: '<svg class="probe-icon" />' },
  });
}

describe('TabBar', () => {
  it('is a named landmark with one labelled button per destination', () => {
    const wrapper = mountBar();

    expect(wrapper.get('nav').attributes('aria-label')).toBe('Areas');
    const tabs = wrapper.findAll('button');
    expect(tabs.map((tab) => tab.text())).toEqual(['Mail', 'Calendar', 'Settings']);
    // The icon is decoration; the label names the target.
    expect(tabs.every((tab) => tab.find('.probe-icon').exists())).toBe(true);
    expect(wrapper.get('.tab-bar__icon').attributes('aria-hidden')).toBe('true');
  });

  it('marks the current tab for assistive technology, not only in colour', () => {
    const wrapper = mountBar('calendar');
    const [mail, calendar] = wrapper.findAll('button');

    expect(calendar!.attributes('aria-current')).toBe('page');
    expect(calendar!.classes()).toContain('tab-bar__item--active');
    expect(mail!.attributes('aria-current')).toBeUndefined();
    // Weight and the icon pill change with it, so the state survives greyscale.
    expect(source).toMatch(/\.tab-bar__item--active \{[^}]*font-weight:/);
    expect(source).toMatch(/\.tab-bar__item--active \.tab-bar__icon \{[^}]*background:/);
  });

  it('tells a new tab from a tap on the current one', async () => {
    const wrapper = mountBar('mail');
    const [mail, calendar] = wrapper.findAll('button');

    await calendar!.trigger('click');
    await mail!.trigger('click');

    expect(wrapper.emitted('select')).toEqual([['calendar']]);
    expect(wrapper.emitted('reselect')).toEqual([['mail']]);
  });

  it('keeps every tab a full touch target and sits on the bottom safe area', () => {
    expect(source).toMatch(/\.tab-bar__item \{[^}]*min-height: var\(--tab-bar-height\)/);
    expect(source).toMatch(/\.tab-bar \{[^}]*var\(--satchel-safe-area-bottom\)/);
    const tokens = readFileSync(join(process.cwd(), 'src/styles/tokens.css'), 'utf8');
    const height = Number(/--tab-bar-height: (\d+)px/.exec(tokens)?.[1]);
    expect(height).toBeGreaterThanOrEqual(44);
  });
});
