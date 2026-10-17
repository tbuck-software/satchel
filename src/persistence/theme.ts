import type { Theme } from '@/types/settings';
import { createPersistedSlot, persistence } from './index';

export type { Theme } from '@/types/settings';

export const THEME_STORAGE_KEY = 'satchel.theme';

const INVALID_THEME = Symbol('invalid-theme');
type StoredTheme = Theme | typeof INVALID_THEME | null;

const themeSlot = createPersistedSlot<StoredTheme>(THEME_STORAGE_KEY, {
  fallback: () => null,
  decode: (stored) => (stored === 'dark' || stored === 'light' ? stored : INVALID_THEME),
  encode: (theme) => {
    if (theme !== 'dark' && theme !== 'light') throw new Error('invalid theme');
    return theme;
  },
});

export function systemTheme(): Theme {
  if (typeof window === 'undefined') return 'dark';

  try {
    return window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

export function readTheme(): Theme {
  const stored = persistence.read(themeSlot);
  return stored === 'dark' || stored === 'light' ? stored : systemTheme();
}

/** Returns whether storage contains any value, including an invalid legacy value. */
export function hasStoredTheme(): boolean {
  return persistence.read(themeSlot) !== null;
}

export function writeTheme(theme: Theme): void {
  persistence.write(themeSlot, theme);
}
