// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { i18n } from '@/i18n';
import IsolatedEmailDocument from './IsolatedEmailDocument.vue';

/*
 * Zoom and the adapted palette used to be written into `srcdoc`, so every
 * zoom step or theme change re-navigated each open mail frame. They are
 * applied to the live document now; only content, CSP and layout rules still
 * produce a new document.
 */

function mountDocument(props: Record<string, unknown> = {}) {
  return mount(IsolatedEmailDocument, {
    props: { html: '<div class="satchel-email-body"><p>Hallo</p></div>', zoom: 100, ...props },
    global: { plugins: [i18n] },
    attachTo: document.body,
  });
}

/** jsdom does not navigate `srcdoc`; give the frame the content a browser would. */
function frameDocument(wrapper: ReturnType<typeof mountDocument>): Document {
  const doc = wrapper.get('iframe').element.contentDocument!;
  if (!doc.body.firstElementChild) doc.body.append(doc.createElement('div'));
  return doc;
}

beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = '';
});

describe('IsolatedEmailDocument', () => {
  it('starts with the zoom and palette in the markup, so the first paint has them', () => {
    const wrapper = mountDocument({ zoom: 120, canvasColor: '#101418', textColor: '#e6e6e6' });
    const srcdoc = wrapper.get('iframe').attributes('srcdoc')!;

    expect(srcdoc).toContain('--satchel-email-zoom: 120%');
    expect(srcdoc).toContain('--satchel-email-canvas: #101418');
    expect(srcdoc).toContain('--satchel-email-ink: #e6e6e6');
    expect(srcdoc).toContain('font-size: var(--satchel-email-zoom, 100%)');
    // The policy still opens the head.
    expect(srcdoc.indexOf('Content-Security-Policy')).toBeLessThan(srcdoc.indexOf('<meta charset'));
    wrapper.unmount();
  });

  it('applies a zoom change to the open document without navigating it', async () => {
    const wrapper = mountDocument();
    const before = wrapper.get('iframe').attributes('srcdoc');
    const doc = frameDocument(wrapper);

    await wrapper.setProps({ zoom: 130 });

    expect(wrapper.get('iframe').attributes('srcdoc')).toBe(before);
    expect(doc.documentElement.style.getPropertyValue('--satchel-email-zoom')).toBe('130%');
    wrapper.unmount();
  });

  it('applies a palette change in place', async () => {
    const wrapper = mountDocument({ canvasColor: '#101418', textColor: '#e6e6e6' });
    const before = wrapper.get('iframe').attributes('srcdoc');
    const doc = frameDocument(wrapper);

    await wrapper.setProps({ canvasColor: '#202428', textColor: '#ffffff' });

    expect(wrapper.get('iframe').attributes('srcdoc')).toBe(before);
    expect(doc.documentElement.style.getPropertyValue('--satchel-email-canvas')).toBe('#202428');
    expect(doc.documentElement.style.getPropertyValue('--satchel-email-ink')).toBe('#ffffff');
    wrapper.unmount();
  });

  it('builds a new document for new content or a new image policy', async () => {
    const wrapper = mountDocument();
    const first = wrapper.get('iframe').attributes('srcdoc');

    await wrapper.setProps({ allowRemoteImages: true });
    const second = wrapper.get('iframe').attributes('srcdoc');
    expect(second).not.toBe(first);
    expect(second).toContain('img-src');

    await wrapper.setProps({ html: '<div class="satchel-email-body"><p>Neu</p></div>', zoom: 90 });
    const third = wrapper.get('iframe').attributes('srcdoc')!;
    expect(third).toContain('<p>Neu</p>');
    expect(third).toContain('--satchel-email-zoom: 90%');
    wrapper.unmount();
  });
});
