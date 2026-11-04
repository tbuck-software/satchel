// @vitest-environment jsdom

import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
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

  it('starts on the system look without storing it', () => {
    const settings = useSettingsStore();

    expect(settings.theme).toBe('light');
    expect(settings.themeFollowsSystem).toBe(true);
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  });

  it('follows the OS while nothing is chosen', async () => {
    const settings = useSettingsStore();

    notifyThemeChange?.({ matches: false });
    await nextTick();

    expect(settings.theme).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  });

  it('keeps a chosen look when the OS changes, even the one already on screen', () => {
    const settings = useSettingsStore();

    settings.setThemePreference('light');
    notifyThemeChange?.({ matches: false });

    expect(settings.themeFollowsSystem).toBe(false);
    expect(settings.theme).toBe('light');
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('light');
  });

  it('counts the theme shortcut as a choice', () => {
    const settings = useSettingsStore();

    settings.toggleTheme();
    notifyThemeChange?.({ matches: true });

    expect(settings.theme).toBe('dark');
    expect(settings.themeFollowsSystem).toBe(false);
  });

  it('goes back to following the OS when System is picked', () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, 'dark');
    const settings = useSettingsStore();
    expect(settings.themeFollowsSystem).toBe(false);

    settings.setThemePreference('system');

    expect(settings.theme).toBe('light');
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
    notifyThemeChange?.({ matches: false });
    expect(settings.theme).toBe('dark');
  });
});
