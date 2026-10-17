// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { THEME_STORAGE_KEY } from '@/persistence/theme';

/**
 * All three window entries have to agree on the theme before their first
 * frame. A window that hard-codes one shows a light user a dark flash, or —
 * in the viewer's case, which never loads the settings store — stays wrong
 * forever.
 */
function stubMatchMedia(prefersLight: boolean) {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('prefers-color-scheme: light') ? prefersLight : false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    onchange: null,
    dispatchEvent: () => false,
  }));
}

describe('window entries apply the stored theme', () => {
  beforeEach(() => {
    vi.resetModules();
    localStorage.clear();
    // The OS says dark, so a passing assertion can only come from storage.
    stubMatchMedia(false);
    localStorage.setItem(THEME_STORAGE_KEY, 'light');
    delete document.documentElement.dataset.theme;
    document.body.innerHTML = '<div id="app"></div>';
  });

  it('viewer', async () => {
    await import('./viewer/main');
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('tray', async () => {
    await import('./tray/main');
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('main window', async () => {
    await import('./main');
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('falls back to the OS preference without a stored choice', async () => {
    localStorage.clear();
    stubMatchMedia(true);
    await import('./viewer/main');
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
