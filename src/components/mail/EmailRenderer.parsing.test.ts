// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import { clearPreparedEmailBodies } from '@/utils/preparedEmailBody';

/*
 * Opening an HTML message parsed it four times: quote extraction, its text
 * fallback, the isolated document, and a plain-text conversion that only fed
 * the decision whether a Markdown option belongs in the toggle — followed by a
 * Markdown lex of up to 250 KB. The HTML view now parses twice and lexes
 * nothing; the Markdown check runs once the reader is in a text mode.
 */

const markdownChecks = vi.fn();

vi.mock('@/utils/safeMarkdown', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/utils/safeMarkdown')>();
  return {
    ...actual,
    hasMeaningfulMarkdown: (text: string) => {
      markdownChecks(text);
      return actual.hasMeaningfulMarkdown(text);
    },
  };
});

const { default: EmailRenderer } = await import('./EmailRenderer.vue');

const stubs = { ExternalLinkDialog: true, LinkDestinationPreview: true };
const markdownishHtml = '<div><h1>Release notes</h1><p># Heading</p><p>- **bold** item</p>'
  + '<p>- second item with `code`</p><p>[link](https://example.com)</p></div>';

function viewModes(wrapper: ReturnType<typeof mount>): string[] {
  const toggle = wrapper.findComponent(SegmentedControl);
  return toggle.exists()
    ? (toggle.props('options') as Array<{ value: string }>).map((option) => option.value)
    : [];
}

describe('EmailRenderer parsing', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    markdownChecks.mockClear();
    vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
  });

  afterEach(() => {
    vi.restoreAllMocks();
    clearPreparedEmailBodies();
  });

  it('parses an HTML message twice and skips the Markdown check in the HTML view', () => {
    const parses = vi.spyOn(DOMParser.prototype, 'parseFromString');
    const wrapper = mount(EmailRenderer, {
      props: { html: markdownishHtml, normalizeQuotes: true, cacheKey: 'parse-1' },
      global: { stubs },
    });

    expect(wrapper.find('iframe').exists()).toBe(true);
    // Quote extraction and the isolated document; no third or fourth pass.
    expect(parses).toHaveBeenCalledTimes(2);
    expect(markdownChecks).not.toHaveBeenCalled();
    expect(viewModes(wrapper)).not.toContain('markdown');
    wrapper.unmount();
  });

  it('offers Markdown for an HTML message once the reader switches to text', async () => {
    const wrapper = mount(EmailRenderer, {
      props: { html: markdownishHtml, normalizeQuotes: true, cacheKey: 'parse-2' },
      global: { stubs },
    });

    wrapper.findComponent(SegmentedControl).vm.$emit('update:modelValue', 'text');
    await wrapper.vm.$nextTick();

    expect(markdownChecks).toHaveBeenCalledTimes(1);
    expect(viewModes(wrapper)).toContain('markdown');
    wrapper.unmount();
  });

  it('still decides Markdown up front for a text-only message', () => {
    const wrapper = mount(EmailRenderer, {
      props: { text: '# Heading\n\n- **bold** item\n- second `code` item\n\n[link](https://example.com)' },
      global: { stubs },
    });

    expect(markdownChecks).toHaveBeenCalled();
    expect(viewModes(wrapper)).toEqual(['markdown', 'text']);
    wrapper.unmount();
  });
});
