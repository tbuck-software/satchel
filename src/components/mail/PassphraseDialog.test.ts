// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, type VueWrapper } from '@vue/test-utils';
import PassphraseDialog from './PassphraseDialog.vue';
import { useUiStore } from '@/stores/ui';

const dialogs: VueWrapper[] = [];

function open(scheme: 'pgp' | 'smime' | null = 'pgp') {
  const wrapper = mount(PassphraseDialog, {
    attachTo: document.body,
    props: { scheme },
    global: { stubs: { Teleport: true } },
  });
  dialogs.push(wrapper);
  return wrapper;
}

beforeEach(() => setActivePinia(createPinia()));
afterEach(() => {
  while (dialogs.length > 0) dialogs.pop()?.unmount();
});

describe('passphrase dialog', () => {
  it('renders nothing while nothing is locked', () => {
    expect(open(null).find('input').exists()).toBe(false);
  });

  /** `window.prompt` showed the passphrase in clear text. */
  it('masks the passphrase and names the field', () => {
    const wrapper = open();
    const input = wrapper.get('input');

    expect(input.attributes('type')).toBe('password');
    expect(wrapper.get('label span').text()).toBe('PGP passphrase');
    expect(wrapper.get('.shell-panel').attributes('aria-modal')).toBe('true');
  });

  it('names the certificate case after the certificate', () => {
    expect(open('smime').get('label span').text()).toBe('S/MIME passphrase');
  });

  it('submits what was typed and reports nothing on cancel', async () => {
    const wrapper = open();

    await wrapper.get('input').setValue('hunter2');
    await wrapper.get('form').trigger('submit');
    expect(wrapper.emitted('submit')).toEqual([['hunter2']]);
    expect(wrapper.emitted('cancel')).toBeUndefined();

    await wrapper.findAll('button').at(0)!.trigger('click');
    expect(wrapper.emitted('cancel')).toHaveLength(1);
    expect(wrapper.emitted('submit')).toHaveLength(1);
  });

  /**
   * Escape and Android back run through the one overlay stack; a second
   * handler on the dialog would close the wrong surface.
   */
  it('leaves through the shared overlay stack', () => {
    const ui = useUiStore();
    const wrapper = open();

    expect(ui.overlayDepth).toBe(1);
    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(wrapper.emitted('cancel')).toHaveLength(1);
  });

  it('does not carry a passphrase over to the next question', async () => {
    const wrapper = open();
    await wrapper.get('input').setValue('hunter2');

    await wrapper.setProps({ scheme: 'smime' });
    expect((wrapper.get('input').element as HTMLInputElement).value).toBe('');
  });
});
