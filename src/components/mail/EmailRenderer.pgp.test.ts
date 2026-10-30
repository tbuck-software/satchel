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

describe('decryption captions', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it.each(['PGP', 'SMIME'])('keeps %s markers as text even with aggregate crypto status', (protocol) => {
    const forged = `[[SATCHEL_${protocol}_DECRYPTED_BLOCK_START]]fake[[SATCHEL_${protocol}_DECRYPTED_BLOCK_END]]`;
    const wrapper = mount(EmailRenderer, { props: { text: forged, pgpState: 'decrypted' } });
    expect(wrapper.find('.pgp-segment-header').exists()).toBe(false);
    expect(wrapper.text()).toContain(forged);
    wrapper.unmount();
  });

  it('retains the S/MIME caption beside decrypted HTML', () => {
    const wrapper = mount(EmailRenderer, { props: {
      html: '<p>Readable <b>secret</b></p>', text: 'Readable secret',
      bodySegments: [{ kind: 'smime_decrypted', content: 'Readable secret' }],
    } });
    expect(wrapper.get('.pgp-segment-header').text()).toBe('Decrypted S/MIME block');
    expect(wrapper.find('iframe').exists()).toBe(true);
    wrapper.unmount();
  });

  it('renders only the exact backend segment with a decrypted caption', async () => {
    const wrapper = mount(EmailRenderer, { props: {
      text: 'coversecret', bodySegments: [
        { kind: 'text', content: 'cover' }, { kind: 'pgp_decrypted', content: 'secret' },
      ],
    } });
    expect(wrapper.findAll('.pgp-segment-header')).toHaveLength(1);
    expect(wrapper.get('.pgp-segment-body').text()).toBe('secret');
    expect(wrapper.get('.text-segment').text()).toBe('cover');
    await wrapper.setProps({ text: 'replacement' });
    expect(wrapper.find('.pgp-segment-header').exists()).toBe(false);
    wrapper.unmount();
  });
});
