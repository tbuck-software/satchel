// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { createPinia } from 'pinia';
import { mount, type VueWrapper } from '@vue/test-utils';
import ComposerSignature from './ComposerSignature.vue';
import type { BackendSignature } from '@/types/composer';

let wrapper: VueWrapper<InstanceType<typeof ComposerSignature>> | null = null;

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = '';
});

const signatures = [
  { id: 'sig-default', name: 'Default' },
  { id: 'sig-brief', name: 'Brief' },
] as BackendSignature[];

function render(props: Partial<InstanceType<typeof ComposerSignature>['$props']> = {}) {
  wrapper = mount(ComposerSignature, {
    attachTo: document.body,
    props: {
      signatures,
      selectedId: 'sig-default',
      variant: 'auto',
      loading: false,
      displayHtml: '<p>Kind regards,<br><strong>Work</strong></p>',
      displayText: 'Kind regards,\nWork',
      overrideActive: false,
      plainText: false,
      disabled: false,
      ...props,
    },
    global: { plugins: [createPinia()] },
  });
  return wrapper;
}

describe('inline signature (phone)', () => {
  it('is plain text under the message, without the SIGNATURE header', () => {
    const view = render({ inline: true });

    expect(view.classes()).toContain('sig-block--inline');
    expect(view.find('.sig-head').exists()).toBe(false);
    expect(view.text()).not.toContain('Signature');
    expect(view.get('.sig-html-edit').text()).toBe('Kind regards,Work');
  });

  it('says nothing when the message goes without a signature', () => {
    const view = render({ inline: true, selectedId: null });

    expect(view.text()).toBe('');
  });

  it('opens its switcher from outside and keeps the signature choice as before', async () => {
    const view = render({ inline: true, overrideActive: true });

    view.vm.openMenu();
    await view.vm.$nextTick();

    const menu = document.body.querySelector('[role="menu"]')!;
    expect(menu).not.toBeNull();
    const items = [...menu.querySelectorAll<HTMLElement>('[role^="menuitem"]')];
    expect(items.map((item) => item.textContent?.trim())).toEqual([
      'No signature', 'Default', 'Brief',
      'Automatic: long first, short afterwards', 'Long version', 'Short version',
      'Discard the signature changes',
    ]);
    items[2]!.click();
    expect(view.emitted('select')).toEqual([['sig-brief']]);

    view.vm.openMenu();
    await view.vm.$nextTick();
    [...document.body.querySelectorAll<HTMLElement>('[role="menuitem"]')].at(-1)!.click();
    expect(view.emitted('reset')).toHaveLength(1);
  });

  it('keeps the desktop block with its header', () => {
    const view = render();

    expect(view.find('.sig-head').exists()).toBe(true);
    expect(view.text()).toContain('Signature');
  });
});
