// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { mount, type VueWrapper } from '@vue/test-utils';
import PgpStatusCard from './PgpStatusCard.vue';
import SmimeStatusCard from './SmimeStatusCard.vue';

let wrapper: VueWrapper | null = null;

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
});

const encoded = (key: string) => JSON.stringify({ key, params: {} });

describe('signature status cards', () => {
  it('reads an S/MIME error the backend sends as a message key', () => {
    wrapper = mount(SmimeStatusCard, {
      props: { state: 'error', errorDetail: encoded('backend.errors.signedMessageMalformed') },
    });
    expect(wrapper.text()).toContain('the message structure does not tie it to the displayed content');
    expect(wrapper.text()).not.toContain('backend.errors');
  });

  it('keeps the engine wording of an S/MIME error that is no key', () => {
    wrapper = mount(SmimeStatusCard, {
      props: { state: 'signature_invalid', errorDetail: 'digest failure' },
    });
    expect(wrapper.text()).toContain('digest failure');
  });

  it('says why no S/MIME signature is shown when the signed content cannot be', () => {
    wrapper = mount(SmimeStatusCard, {
      props: { state: 'error', errorDetail: encoded('backend.errors.signedContentUnavailable') },
    });
    expect(wrapper.text()).toContain('signed content of this message cannot be shown');
  });

  /** Checking the same structure again cannot tie it to the content. */
  it('offers no retry for a structure that does not bind its signature', () => {
    wrapper = mount(PgpStatusCard, {
      props: { state: 'error', errorDetail: encoded('backend.errors.signedMessageMalformed') },
    });
    const labels = wrapper.findAll('button').map((button) => button.text());
    expect(labels).not.toContain('Try again');
    expect(labels).toContain('Close');
  });

  it('still offers a retry when a check failed for another reason', () => {
    wrapper = mount(PgpStatusCard, {
      props: { state: 'error', errorDetail: encoded('backend.errors.pgpVerificationFailed') },
    });
    expect(wrapper.findAll('button').map((button) => button.text())).toContain('Try again');
  });
});
