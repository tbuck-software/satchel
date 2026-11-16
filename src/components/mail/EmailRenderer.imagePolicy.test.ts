// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

/*
 * PERF-10. `safeHtml` and `blockedImagesCount` each called `blockRemoteImages`
 * on the same input and threw away half of the return value, so every render
 * of a blocked newsletter ran the whole block pass twice. The pass now runs
 * inside the parse that builds the isolated document; the counter below is the
 * acceptance criterion.
 */

const calls = vi.fn();

vi.mock('@/composables/useImagePolicy', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/composables/useImagePolicy')>();
  return {
    ...actual,
    blockRemoteImagesInDocument: (doc: Document) => {
      calls(doc);
      return actual.blockRemoteImagesInDocument(doc);
    },
  };
});

const { default: EmailRenderer } = await import('./EmailRenderer.vue');
const { blockRemoteImages, useImagePolicy } = await import('@/composables/useImagePolicy');

const stubs = { ExternalLinkDialog: true, LinkDestinationPreview: true };

function newsletterHtml(imageCount: number): string {
  const rows = Array.from(
    { length: imageCount },
    (_, index) =>
      `<tr><td style="background: url(https://cdn.example.com/bg${index}.png)">`
      + `<img src="https://cdn.example.com/pixel${index}.png" srcset="https://cdn.example.com/2x${index}.png 2x">`
      + `<p>${'Fülltext '.repeat(20)}</p></td></tr>`,
  ).join('');
  return `<table>${rows}</table>`;
}

describe('EmailRenderer remote image handling', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    useImagePolicy().setMode('allowlist');
    calls.mockClear();
    vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
  });

  it('evaluates the block pass once per render, not twice', () => {
    const html = newsletterHtml(5);
    const wrapper = mount(EmailRenderer, {
      props: { html, senderEmail: 'newsletter@unbekannt.example' },
      global: { stubs },
    });

    // Both consumers read the same result. Was two.
    expect(calls).toHaveBeenCalledTimes(1);
    // …and the banner still reports the count from that one result.
    const expected = blockRemoteImages(html).blockedCount;
    expect(expected).toBeGreaterThan(0);
    expect(wrapper.text()).toContain(String(expected));

    wrapper.unmount();
  });

  it('does not evaluate the block pass at all when images are allowed', () => {
    useImagePolicy().setMode('all');
    calls.mockClear();

    const wrapper = mount(EmailRenderer, {
      props: {
        html: newsletterHtml(5),
        senderEmail: 'newsletter@unbekannt.example',
      },
      global: { stubs },
    });

    expect(calls).not.toHaveBeenCalled();

    wrapper.unmount();
  });
});
