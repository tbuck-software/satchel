import { expect, it, vi } from 'vitest';

/*
 * Lazy code runs after an entry created its i18n instance. If one ever ran
 * before, resolving the chunk load would let the module render raw keys;
 * the load has to fail instead.
 */
it('rejects a chunk load when no i18n instance is active', async () => {
  vi.resetModules();
  vi.doMock('./runtime', () => ({
    activeI18n: () => null,
    addLocaleLoader: () => {},
    wantedLocales: () => ['en'],
  }));
  const { withMessages } = await import('./messageChunks');
  const load = vi.fn(async () => ({}));
  const en = vi.fn(async () => ({ default: { calendar: { title: 'Calendar' } } }));

  await expect(withMessages('views.CalendarView', async () => ({ en, de: en }), load))
    .rejects.toThrow('no i18n instance for message chunk views.CalendarView');
  expect(load).not.toHaveBeenCalled();
  expect(en).not.toHaveBeenCalled();
  vi.doUnmock('./runtime');
});
