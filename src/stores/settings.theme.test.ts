// @vitest-environment jsdom

import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { THEME_STORAGE_KEY } from '@/persistence/theme';
import { useSettingsStore } from './settings';

describe('settings theme persistence', () => {
  let notifyThemeChange: ((event: { matches: boolean }) => void) | undefined;

  beforeEach(() => {
    window.localStorage.clear();
    delete document.documentElement.dataset.theme;
    notifyThemeChange = undefined;
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => ({
        matches: true,
        addEventListener: (
          _type: string,
          listener: (event: { matches: boolean }) => void,
        ) => {
          notifyThemeChange = listener;
        },
      })),
    });
    setActivePinia(createPinia());
  });

  it('applies and persists the resolved theme during store startup', () => {
    const settings = useSettingsStore();

    expect(settings.theme).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('light');
  });

  it('keeps the initialized theme when the OS preference changes', () => {
    const settings = useSettingsStore();

    notifyThemeChange?.({ matches: false });

    expect(settings.theme).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
