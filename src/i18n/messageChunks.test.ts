import { afterEach, describe, expect, it, vi } from 'vitest';
import type { LocaleMessageValue } from 'vue-i18n';
import { createLazyAppI18n } from './create';
import { withMessages, type MessageChunkLoaders } from './messageChunks';
import { applyLocalePreference, setActiveI18n } from './runtime';
import { i18n } from './index';

type Catalogue = Record<string, LocaleMessageValue>;

afterEach(() => setActiveI18n(i18n.global as never));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function chunk(en: Catalogue, de: Catalogue) {
  return {
    en: vi.fn(async () => ({ default: en })),
    de: vi.fn(async () => ({ default: de })),
  } satisfies MessageChunkLoaders;
}

/** The build hands `withMessages` a lookup into its chunk table. */
const table = (loaders: MessageChunkLoaders) => async () => loaders;

async function startup() {
  const { instance, ready } = createLazyAppI18n({
    en: async () => ({ default: { greeting: 'Hello' } }),
    de: async () => ({ default: { greeting: 'Hallo' } }),
  });
  await ready;
  return instance.global;
}

describe('message chunks', () => {
  it('merges the chunk before the lazy module loads', async () => {
    const t = await startup();
    const settings = chunk({ settings: { title: 'Settings' } }, { settings: { title: 'Einstellungen' } });
    const seen: string[] = [];

    await withMessages('views.SettingsView', table(settings), async () => {
      // What a lazy component's setup or module top level would read first.
      seen.push(t.t('settings.title'));
      return {};
    });

    expect(seen).toEqual(['Settings']);
    expect(settings.en).toHaveBeenCalledTimes(1);
    expect(settings.de).not.toHaveBeenCalled();
  });

  it('does not start the import until the chunk is in', async () => {
    await startup();
    const pending = deferred<{ default: Catalogue }>();
    const load = vi.fn(async () => ({}));
    const loading = withMessages('views.CalendarView', table({
      en: () => pending.promise,
      de: async () => ({ default: {} }),
    }), load);

    await Promise.resolve();
    expect(load).not.toHaveBeenCalled();
    pending.resolve({ default: { calendar: { title: 'Calendar' } } });
    await loading;
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('loads every chunk loaded so far in the new locale before switching', async () => {
    const t = await startup();
    const settings = chunk({ settings: { title: 'Settings' } }, { settings: { title: 'Einstellungen' } });
    await withMessages('views.SettingsView', table(settings), async () => ({}));

    await applyLocalePreference('de', false);

    expect(t.locale.value).toBe('de');
    expect(t.t('greeting')).toBe('Hallo');
    expect(t.t('settings.title')).toBe('Einstellungen');
    expect(settings.de).toHaveBeenCalledTimes(1);
  });

  it('loads a chunk in both locales while a switch is still loading', async () => {
    const german = deferred<{ default: Catalogue }>();
    const { instance, ready } = createLazyAppI18n({
      en: async () => ({ default: { greeting: 'Hello' } }),
      de: () => german.promise,
    });
    await ready;
    const switching = applyLocalePreference('de', false);
    const outbox = chunk({ outbox: { title: 'Outbox' } }, { outbox: { title: 'Postausgang' } });

    await withMessages('views.OutboxView', table(outbox), async () => ({}));
    german.resolve({ default: { greeting: 'Hallo' } });
    await switching;

    expect(outbox.en).toHaveBeenCalledTimes(1);
    expect(outbox.de).toHaveBeenCalledTimes(1);
    expect(instance.global.t('outbox.title')).toBe('Postausgang');
  });

  it('keeps chunk strings when the base catalogue of that locale arrives later', async () => {
    const german = deferred<{ default: Catalogue }>();
    const { instance, ready } = createLazyAppI18n({
      en: async () => ({ default: { greeting: 'Hello' } }),
      de: () => german.promise,
    });
    await ready;
    const switching = applyLocalePreference('de', false);
    await withMessages('views.OutboxView', table(chunk({ outbox: { title: 'Outbox' } }, { outbox: { title: 'Postausgang' } })), async () => ({}));
    german.resolve({ default: { greeting: 'Hallo' } });
    await switching;

    expect(instance.global.t('greeting')).toBe('Hallo');
    expect(instance.global.t('outbox.title')).toBe('Postausgang');
  });

  it('loads each chunk once per locale', async () => {
    await startup();
    const contacts = chunk({ contacts: { title: 'Contacts' } }, { contacts: { title: 'Kontakte' } });
    await withMessages('views.ContactsView', table(contacts), async () => ({}));
    await withMessages('views.ContactsView', table(contacts), async () => ({}));
    await applyLocalePreference('de', false);
    await applyLocalePreference('en', false);
    await applyLocalePreference('de', false);

    expect(contacts.en).toHaveBeenCalledTimes(1);
    expect(contacts.de).toHaveBeenCalledTimes(1);
  });

  it('fails the import when its chunk cannot load, and tries again next time', async () => {
    const t = await startup();
    const load = vi.fn(async () => ({}));
    const en = vi.fn()
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValue({ default: { cleanup: { title: 'Clean up' } } });

    await expect(withMessages('views.CleanupView', table({ en, de: async () => ({ default: {} }) }), load)).rejects.toThrow('offline');
    expect(load).not.toHaveBeenCalled();

    await withMessages('views.CleanupView', table({ en, de: async () => ({ default: {} }) }), load);
    expect(load).toHaveBeenCalledTimes(1);
    expect(t.t('cleanup.title')).toBe('Clean up');
  });

  it('stops loading chunks in a locale whose catalogue failed', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    createLazyAppI18n({
      en: async () => ({ default: { greeting: 'Hello' } }),
      de: async () => { throw new Error('missing'); },
    });
    await applyLocalePreference('en', false);
    await applyLocalePreference('de', false);
    const drafts = chunk({ drafts: { title: 'Drafts' } }, { drafts: { title: 'Entwürfe' } });
    await withMessages('views.DraftsView', table(drafts), async () => ({}));

    expect(drafts.en).toHaveBeenCalledTimes(1);
    expect(drafts.de).not.toHaveBeenCalled();
    warn.mockRestore();
  });
});
