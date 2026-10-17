// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import MessageSecurityBadge from './MessageSecurityBadge.vue';

afterEach(() => {
  document.body.innerHTML = '';
});

describe('message security badge', () => {
  it('shows an icon-only signature badge and requests the details on click', async () => {
    const wrapper = mount(MessageSecurityBadge, {
      attachTo: document.body,
      props: { smimeState: 'signature_valid', authentication: 'pass' },
    });

    const button = wrapper.get('button');
    expect(button.text()).toBe('');
    expect(button.attributes('title')).toBe('Signature valid');
    expect(button.attributes('aria-label')).toBe('Signature valid. Show details');
    expect(button.classes()).toContain('security-badge--success');

    await button.trigger('click');

    expect(wrapper.emitted('showDetails')).toHaveLength(1);
  });

  it('lets a failed authentication outrank a valid signature', () => {
    const wrapper = mount(MessageSecurityBadge, {
      props: { smimeState: 'signature_valid', authentication: 'fail' },
    });

    const button = wrapper.get('button');
    expect(button.attributes('title')).toBe('Authentication failed');
    expect(button.classes()).toContain('security-badge--danger');
  });

  it('keeps a plain authentication pass muted', () => {
    const wrapper = mount(MessageSecurityBadge, {
      props: { authentication: 'pass' },
    });

    expect(wrapper.get('button').classes()).toContain('security-badge--muted');
  });

  it('renders nothing without any security signal', () => {
    const wrapper = mount(MessageSecurityBadge, {
      props: { authentication: 'none' },
    });

    expect(wrapper.find('button').exists()).toBe(false);
  });

  it('keeps a signature without a trust chain apart from a checked one', () => {
    const unchained = mount(MessageSecurityBadge, {
      props: { smimeState: 'signature_unchained' },
    });
    const checked = mount(MessageSecurityBadge, {
      props: { smimeState: 'signature_valid' },
    });

    expect(unchained.get('button').attributes('title')).toBe('Certificate unconfirmed');
    expect(unchained.get('button').classes()).toContain('security-badge--warning');
    expect(unchained.get('button').classes()).not.toContain('security-badge--success');
    expect(checked.get('button').classes()).toContain('security-badge--success');
  });

  it('keeps a signature from a foreign mailbox apart from a checked one', () => {
    const mismatch = mount(MessageSecurityBadge, {
      props: {
        smimeState: 'signature_identity_mismatch',
        smimeSigner: 'attacker@example.test',
        fromAddress: 'board@firma.test',
      },
    });

    const button = mismatch.get('button');
    expect(button.attributes('title')).toBe('Signed by attacker@example.test, not by board@firma.test');
    expect(button.classes()).toContain('security-badge--warning');
    expect(button.classes()).not.toContain('security-badge--success');
  });

  it('names the PGP signer and its fingerprint in the tooltip', () => {
    const wrapper = mount(MessageSecurityBadge, {
      props: {
        pgpState: 'signature_identity_mismatch',
        pgpSigner: 'Attacker <attacker@example.test>',
        pgpSignerFingerprint: 'ABCD 1234',
        fromAddress: 'board@firma.test',
      },
    });

    const button = wrapper.get('button');
    expect(button.attributes('title')).toBe(
      'Signed by Attacker <attacker@example.test> (ABCD 1234), not by board@firma.test',
    );
    expect(button.classes()).toContain('security-badge--warning');
  });

  it('labels a detected but unverified signature as such', () => {
    const wrapper = mount(MessageSecurityBadge, {
      props: { smimeState: 'signature_unverified' },
    });

    const button = wrapper.get('button');
    expect(button.attributes('aria-label')).toBe('S/MIME signature detected. Show details');
    expect(button.classes()).toContain('security-badge--warning');
  });
});

it('warns about partial coverage even if header authentication passes', () => {
  for (const props of [{ smimeState: 'signature_partial' as const }, { pgpState: 'signature_partial' as const }]) {
    const wrapper = mount(MessageSecurityBadge, { props: { ...props, authentication: 'pass' } });
    expect(wrapper.find('button').classes()).toContain('security-badge--warning');
    expect(wrapper.find('button').attributes('title')).toContain('Partially signed');
  }
});
