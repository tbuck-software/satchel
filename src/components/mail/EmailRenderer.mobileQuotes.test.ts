// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import EmailRenderer from './EmailRenderer.vue';
import IsolatedEmailDocument from './IsolatedEmailDocument.vue';

describe('EmailRenderer quoted history on touch screens', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
  });

  it('lets vertical touch scrolling pass through quoted HTML documents', async () => {
    const wrapper = mount(EmailRenderer, {
      props: {
        html: '<div>Aktuelle Antwort</div><blockquote type="cite">Frühere Nachricht</blockquote>',
        senderEmail: 'mike@example.com',
        normalizeQuotes: true,
      },
      global: {
        stubs: {
          ExternalLinkDialog: true,
          LinkDestinationPreview: true,
        },
      },
    });

    const details = wrapper.get('details');
    (details.element as HTMLDetailsElement).open = true;
    await details.trigger('toggle');

    const quoteDocument = wrapper.findAllComponents(IsolatedEmailDocument)
      .find((document) => document.props('passThroughTouchScroll') === true);
    expect(quoteDocument).toBeDefined();
    expect(quoteDocument?.props('passThroughTouchScroll')).toBe(true);

    wrapper.unmount();
  });

  it('marks only opted-in isolated documents as touch-scroll pass-through', () => {
    const wrapper = mount(IsolatedEmailDocument, {
      props: {
        html: '<div>Zitat</div>',
        passThroughTouchScroll: true,
      },
    });

    expect(wrapper.get('.satchel-email-host').classes())
      .toContain('satchel-email-host--pass-through-touch-scroll');

    wrapper.unmount();
  });
});
