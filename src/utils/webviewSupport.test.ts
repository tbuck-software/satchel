// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderUnsupportedRuntimeNotice, supportsRequiredCss } from './webviewSupport';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('runtime capability floor', () => {
  it('accepts an engine that can mix colours', () => {
    vi.stubGlobal('CSS', { supports: () => true });
    expect(supportsRequiredCss()).toBe(true);
  });

  it('rejects an engine without color-mix', () => {
    // What Android System WebView 101 does: it knows CSS.supports, and answers
    // no. The design system is unusable there, not merely plainer.
    vi.stubGlobal('CSS', { supports: (_p: string, value: string) => !value.includes('color-mix') });
    expect(supportsRequiredCss()).toBe(false);
  });

  it('rejects an engine without CSS.supports at all', () => {
    vi.stubGlobal('CSS', undefined);
    expect(supportsRequiredCss()).toBe(false);
  });

  it('explains the fix without relying on the token system', () => {
    const root = document.createElement('div');
    renderUnsupportedRuntimeNotice(root);

    expect(root.textContent).toContain('Android System WebView');
    // No custom properties: the notice has to render on the engine that just
    // failed to render them.
    expect(root.innerHTML).not.toContain('var(--');
    expect(root.innerHTML).not.toContain('color-mix');
  });
});
