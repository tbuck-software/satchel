// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import EmailRenderer from './EmailRenderer.vue';

const text = '-----BEGIN PGP MESSAGE-----\nciphertext\n-----END PGP MESSAGE-----';

describe('encrypted block actions', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it.each([null, 'key_available_locked', 'error'] as const)(
    'offers decryption with a handler even when the PGP state is %s', async (pgpState) => {
      const wrapper = mount(EmailRenderer, { props: { text, pgpState, canDecrypt: true } });
      const action = wrapper.get('.pgp-segment-action');
      await action.trigger('click');
      expect(wrapper.emitted('request-unlock')).toHaveLength(1);
      await wrapper.setProps({ decrypting: true });
      expect((action.element as HTMLButtonElement).disabled).toBe(true);
      wrapper.unmount();
    },
  );

  it('does not offer an inert action in a viewer without decryption support', () => {
    const wrapper = mount(EmailRenderer, { props: { text } });
    expect(wrapper.find('.pgp-segment-action').exists()).toBe(false);
    expect(wrapper.text()).toContain('Settings → OpenPGP');
    wrapper.unmount();
  });
});
