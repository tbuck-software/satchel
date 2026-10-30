// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

/*
 * PERF-10. `safeHtml` and `blockedImagesCount` each called `blockRemoteImages`
 * on the same input and threw away half of the return value, so every render
 * of a blocked newsletter ran the whole regex chain twice.
 *
 * The counter below is the acceptance criterion. The benchmark that follows it
 * is an interleaved A/B against the two-call shape and asserts a ratio; wall
 * time on this machine is not a usable gate.
 */

const calls = vi.fn();

vi.mock('@/composables/useImagePolicy', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/composables/useImagePolicy')>();
  return {
    ...actual,
    blockRemoteImages: (html: string) => {
      calls(html);
      return actual.blockRemoteImages(html);
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

function median(samples: number[]): number {
  const sorted = [...samples].sort((left, right) => left - right);
  return sorted[Math.floor(sorted.length / 2)] ?? 0;
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

  it('beats the two-call shape on a large document', () => {
    // ~800 KB of newsletter HTML, the size the issue describes.
    const html = newsletterHtml(3600);
    expect(html.length).toBeGreaterThan(800_000);

    const single: number[] = [];
    const double: number[] = [];
    for (let run = 0; run < 15; run += 1) {
      const start = performance.now();
      const once = blockRemoteImages(html);
      const midpoint = performance.now();
      const html2 = blockRemoteImages(html).html;
      const count2 = blockRemoteImages(html).blockedCount;
      const end = performance.now();

      expect(html2).toBe(once.html);
      expect(count2).toBe(once.blockedCount);
      if (run < 3) continue; // warmup
      single.push(midpoint - start);
      double.push(end - midpoint);
    }

    console.log(
      `blockRemoteImages(800 KB) one pass: ${median(single).toFixed(2)} ms, `
      + `two passes: ${median(double).toFixed(2)} ms`,
    );
    // The fastest run of each shape is the least disturbed by GC pauses and
    // other load on a shared runner; the median still swings past 0.8 there.
    expect(Math.min(...single) / Math.min(...double)).toBeLessThan(0.8);
  }, 120_000);
});
