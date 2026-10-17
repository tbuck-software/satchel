import { afterEach, expect, it, vi } from 'vitest';
import { createLazyAppI18n } from './create';
import { applyLocalePreference, setActiveI18n } from './runtime';
import { i18n } from './index';

afterEach(() => setActiveI18n(i18n.global as never));

it('loads only the startup locale and caches a later switch', async () => {
  const en = vi.fn(async () => ({ default: { greeting: 'Hello' } }));
  const de = vi.fn(async () => ({ default: { greeting: 'Hallo' } }));
  const { instance, ready } = createLazyAppI18n({ en, de });
  await ready;
  expect(en).toHaveBeenCalledTimes(1);
  expect(de).not.toHaveBeenCalled();
  expect(instance.global.t('greeting')).toBe('Hello');
  await applyLocalePreference('de', false);
  expect(instance.global.t('greeting')).toBe('Hallo');
  await applyLocalePreference('en', false);
  await applyLocalePreference('de', false);
  expect(de).toHaveBeenCalledTimes(1);
});

it('does not apply an earlier locale request after a newer preference', async () => {
  let resolve!: (module: { default: { greeting: string } }) => void;
  const { instance, ready } = createLazyAppI18n({
    en: async () => ({ default: { greeting: 'Hello' } }),
    de: () => new Promise(done => { resolve = done; }),
  });
  await ready;
  const pending = applyLocalePreference('de', false);
  await applyLocalePreference('en', false);
  resolve({ default: { greeting: 'Hallo' } });
  await pending;
  expect(instance.global.t('greeting')).toBe('Hello');
});
