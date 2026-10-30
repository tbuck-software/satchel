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

  it('says a quoted block was not decrypted instead of offering decryption again', () => {
    const block = '> -----BEGIN PGP MESSAGE-----\n> ciphertext\n> -----END PGP MESSAGE-----\n';
    const segments = [
      { kind: 'text' as const, content: 'Forwarding it back.\n' },
      { kind: 'pgp_undecrypted' as const, content: block },
    ];
    const wrapper = mount(EmailRenderer, { props: {
      text: `Forwarding it back.\n${block}`, canDecrypt: true, bodySegments: segments,
      bodyProvenance: { html: null, attachments: [], segments },
    } });
    expect(wrapper.find('.pgp-segment-action').exists()).toBe(false);
    expect(wrapper.get('.pgp-segment-header').text()).toBe('Encrypted OpenPGP block');
    expect(wrapper.get('.pgp-segment-hint').text()).toContain('None of your unlocked keys can decrypt this block');
    expect(wrapper.get('.pgp-segment-raw').text()).toContain('ciphertext');
    expect(wrapper.get('.text-segment').text()).toBe('Forwarding it back.');
    wrapper.unmount();
  });

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
      bodyProvenance: { html: '<p>Readable <b>secret</b></p>', attachments: [], segments: [{ kind: 'smime_decrypted', content: 'Readable secret' }] },
      bodySegments: [{ kind: 'smime_decrypted', content: 'Readable secret' }],
    } });
    expect(wrapper.get('.pgp-segment-header').text()).toBe('Decrypted S/MIME block');
    expect(wrapper.find('iframe').exists()).toBe(true);
    wrapper.unmount();
  });

  it('keeps the caption over HTML decrypted from several parts', () => {
    const segments = [
      { kind: 'smime_decrypted' as const, content: 'Part one' },
      { kind: 'text' as const, content: '\n\n' },
      { kind: 'smime_decrypted' as const, content: 'Part two' },
    ];
    const html = '<p>Part <b>one</b></p><p>Part two</p>';
    const wrapper = mount(EmailRenderer, { props: {
      html, text: 'Part one\n\nPart two', bodySegments: segments,
      bodyProvenance: { html, attachments: [], segments },
    } });
    expect(wrapper.get('.pgp-segment-header').text()).toBe('Decrypted S/MIME block');
    expect(wrapper.find('iframe').exists()).toBe(true);
    wrapper.unmount();
  });

  it('shows each decrypted part as its own captioned segment in text mode', () => {
    const segments = [
      { kind: 'smime_decrypted' as const, content: 'Unsigned cover' },
      { kind: 'text' as const, content: '\n\n' },
      { kind: 'smime_decrypted' as const, content: 'Signed paragraph' },
    ];
    const wrapper = mount(EmailRenderer, { props: {
      text: 'Unsigned cover\n\nSigned paragraph', bodySegments: segments,
      bodyProvenance: { html: null, attachments: [], segments },
    } });
    expect(wrapper.findAll('.pgp-segment-body').map((part) => part.text()))
      .toEqual(['Unsigned cover', 'Signed paragraph']);
    wrapper.unmount();
  });

  it('renders only the exact backend segment with a decrypted caption', async () => {
    const wrapper = mount(EmailRenderer, { props: {
      text: 'coversecret', bodyProvenance: { html: null, attachments: [], segments: [{ kind: 'text', content: 'cover' }, { kind: 'pgp_decrypted', content: 'secret' }] }, bodySegments: [
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


describe('decoded HTML provenance', () => {
  beforeEach(() => setActivePinia(createPinia()));
  const html = '<p>Readable <b>secret</b></p>';
  const text = 'Readable secret';
  const bodySegments = [{ kind: 'smime_decrypted' as const, content: text }];
  const bodyProvenance = { html, attachments: [], segments: bodySegments };

  it('removes captions when HTML changes with identical text and segments', async () => {
    const wrapper = mount(EmailRenderer, { props: { html, text, bodyProvenance, bodySegments } });
    expect(wrapper.findAll('.pgp-segment-header')).toHaveLength(1);
    await wrapper.setProps({ html: '<p><b>Substituted HTML</b></p>' });
    expect(wrapper.get('iframe').attributes('srcdoc')).toContain('Substituted HTML');
    expect(wrapper.findAll('.pgp-segment-header')).toHaveLength(0);
    wrapper.unmount();
  });

  it('captions empty-text image HTML from a complete decoded body', () => {
    const html = '<img src="cid:synthetic" width="400" height="300">';
    const wrapper = mount(EmailRenderer, { props: {
      html, text: '', bodyProvenance: { html, attachments: [], segments: [{ kind: 'smime_decrypted', content: '' }] },
      bodySegments: [{ kind: 'smime_decrypted', content: '' }],
    } });
    expect(wrapper.find('iframe').exists()).toBe(true);
    expect(wrapper.get('.pgp-segment-header').text()).toBe('Decrypted S/MIME block');
    wrapper.unmount();
  });

  it('keeps the selector reachable through HTML to Text to HTML', async () => {
    const wrapper = mount(EmailRenderer, { props: { html, text, bodyProvenance, bodySegments } });
    wrapper.getComponent({ name: 'SegmentedControl' }).vm.$emit('update:modelValue', 'text');
    await wrapper.vm.$nextTick();
    expect(wrapper.get('.pgp-segment-body').text()).toBe(text);
    wrapper.getComponent({ name: 'SegmentedControl' }).vm.$emit('update:modelValue', 'html');
    await wrapper.vm.$nextTick();
    expect(wrapper.find('iframe').exists()).toBe(true);
    wrapper.unmount();
  });
});


it('does not caption independently supplied HTML with text-only segments', () => {
  setActivePinia(createPinia());
  const wrapper = mount(EmailRenderer, { props: {
    html: '<p><b>Ordinary HTML</b></p>', text: 'secret',
    bodySegments: [{ kind: 'smime_decrypted', content: 'secret' }],
  } });
  expect(wrapper.findAll('.pgp-segment-header')).toHaveLength(0);
  wrapper.unmount();
});

it('invalidates captions when published attachments change', async () => {
  setActivePinia(createPinia());
  const html = '<p><b>Secret</b><img src="cid:part"></p>';
  const attachment = { id: 'att-1', filename: 'photo.png', contentType: 'image/png', size: 3,
    contentId: 'part', isInline: true, cachedPath: '/cache/photo.png', dataUrl: 'data:image/png;base64,AQID' };
  const binding = { id: attachment.id, filename: attachment.filename, content_type: attachment.contentType,
    size: attachment.size, content_id: attachment.contentId, is_inline: attachment.isInline,
    cached_path: attachment.cachedPath, data_url: attachment.dataUrl };
  const wrapper = mount(EmailRenderer, { props: {
    html, text: 'Secret', bodySegments: [{ kind: 'smime_decrypted', content: 'Secret' }],
    bodyProvenance: { html, attachments: [binding], segments: [{ kind: 'smime_decrypted', content: 'Secret' }] }, attachments: [attachment],
  } });
  expect(wrapper.findAll('.pgp-segment-header')).toHaveLength(1);
  for (const replacement of [
    { ...attachment, filename: 'other.png' }, { ...attachment, cachedPath: '/other.png' },
    { ...attachment, dataUrl: 'data:image/png;base64,BAUG' },
  ]) {
    await wrapper.setProps({ attachments: [replacement] });
    expect(wrapper.findAll('.pgp-segment-header')).toHaveLength(0);
  }
  wrapper.unmount();
});


it('drops the caption when reordered attachments change which image a shared Content-ID shows', async () => {
  setActivePinia(createPinia());
  const html = '<p>Secret</p><img src="cid:shared">';
  const segments = [{ kind: 'smime_decrypted' as const, content: 'Secret' }];
  const first = { id: 'a', filename: 'a.png', contentType: 'image/png', size: 3, contentId: 'shared',
    isInline: true, cachedPath: '/cache/a.png', dataUrl: 'data:image/png;base64,AQID' };
  const second = { ...first, id: 'b', filename: 'b.png', cachedPath: '/cache/b.png', dataUrl: 'data:image/png;base64,BAUG' };
  const binding = (part: typeof first) => ({ id: part.id, filename: part.filename, content_type: part.contentType,
    size: part.size, content_id: part.contentId, is_inline: part.isInline, cached_path: part.cachedPath, data_url: part.dataUrl });
  const wrapper = mount(EmailRenderer, { props: {
    html, text: 'Secret', bodySegments: segments, cacheKey: 'attachment-order',
    bodyProvenance: { html, segments, attachments: [binding(first), binding(second)] }, attachments: [first, second],
  } });
  const before = wrapper.get('iframe').attributes('srcdoc');
  expect(wrapper.findAll('.pgp-segment-header')).toHaveLength(1);
  await wrapper.setProps({ attachments: [second, first] });
  expect(wrapper.get('iframe').attributes('srcdoc')).not.toBe(before);
  expect(wrapper.findAll('.pgp-segment-header')).toHaveLength(0);
  wrapper.unmount();
});

it('rejects redistributed segments even when their concatenated text is unchanged', () => {
  setActivePinia(createPinia());
  const wrapper = mount(EmailRenderer, { props: {
    text: 'coversecret', bodySegments: [{ kind: 'smime_decrypted', content: 'coversecret' }],
    bodyProvenance: { html: null, attachments: [], segments: [
      { kind: 'text', content: 'cover' }, { kind: 'smime_decrypted', content: 'secret' },
    ] },
  } });
  expect(wrapper.findAll('.pgp-segment-header')).toHaveLength(0);
  wrapper.unmount();
});
