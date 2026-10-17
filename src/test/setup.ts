import { config } from '@vue/test-utils';
import { i18n, i18nReady } from '@/i18n';

/*
 * Node 26 exposes an undefined `localStorage` unless `--localstorage-file` is
 * configured. Vitest copies that property into jsdom, shadowing jsdom's own
 * implementation. Keep the browser contract available without requiring a
 * machine-specific Node flag.
 */
if (typeof window !== 'undefined' && !window.localStorage) {
  const values = new Map<string, string>();
  const storage: Storage = {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => Array.from(values.keys())[index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(String(key), String(value)),
  };

  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    value: storage,
  });
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: storage,
  });
}

/*
 * Every mounted component resolves `$t` against the real catalogue, and
 * importing the instance also registers it for `@/i18n/runtime` — so stores and
 * utilities translate in tests exactly as they do in the app. Tests assert
 * English: `en` is the default locale and no test sets another one.
 */
config.global.plugins = [...(config.global.plugins ?? []), i18n];

await i18nReady;
