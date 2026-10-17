import { afterEach, describe, expect, it } from 'vitest';
import {
  setPersistenceStorageProvider,
  type StorageLike,
} from './index';
import {
  settingsPersistence,
} from './settings';

const SETTINGS_STORAGE_KEYS = {
  onboardingComplete: 'satchel.onboardingComplete',
  singleKeyShortcuts: 'satchel.singleKeyShortcuts',
  autoDarkHtmlMails: 'satchel.autoDarkHtmlMails',
  preferPlainTextReading: 'satchel.preferPlainTextReading',
  readerZoom: 'satchel.readerZoom',
  composePlainText: 'satchel.composePlainText',
  collapseQuotedHistory: 'satchel.collapseQuotedHistory',
  attachmentOpenBehavior: 'satchel.attachmentOpenBehavior',
  inboxGroupingMode: 'satchel.inboxGroupingMode',
  swipeActions: 'satchel.swipeActions',
  pinnedContacts: 'satchel.pinnedContacts',
  inboxSenderBuckets: 'satchel.inboxSenderBuckets',
  acceptedSenders: 'satchel.acceptedSenders',
  acceptedDomains: 'satchel.acceptedDomains',
  trustedLinkHosts: 'satchel.trustedLinkHosts',
  senderReviewStartedAt: 'satchel.senderReviewStartedAt',
  dismissedNewSenders: 'satchel.dismissedNewSenders',
  keybindings: 'satchel.keybindings',
} as const;

function memoryStorage(initial: Record<string, string> = {}): StorageLike & {
  values: Map<string, string>;
} {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}

describe('settings persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
  });

  it('requires explicit consent before sending contact identifiers to third parties', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);
    expect(settingsPersistence.thirdPartyContactAvatars.read()).toBe(false);
    settingsPersistence.thirdPartyContactAvatars.write(true);
    expect(settingsPersistence.thirdPartyContactAvatars.read()).toBe(true);
    settingsPersistence.thirdPartyContactAvatars.write(false);
    expect(settingsPersistence.thirdPartyContactAvatars.read()).toBe(false);
  });

  it('keeps stable primitive keys, defaults, and legacy boolean semantics', () => {
    const storage = memoryStorage({
      [SETTINGS_STORAGE_KEYS.onboardingComplete]: 'unexpected',
      [SETTINGS_STORAGE_KEYS.autoDarkHtmlMails]: 'unexpected',
      [SETTINGS_STORAGE_KEYS.readerZoom]: '250',
      [SETTINGS_STORAGE_KEYS.attachmentOpenBehavior]: 'system',
      [SETTINGS_STORAGE_KEYS.inboxGroupingMode]: 'classic',
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(settingsPersistence.onboardingComplete.read()).toBe(false);
    expect(settingsPersistence.autoDarkHtmlMails.read()).toBe(false);
    expect(settingsPersistence.readerZoom.read()).toBe(250);
    expect(settingsPersistence.attachmentOpenBehavior.read()).toBe('system');
    expect(settingsPersistence.inboxGroupingMode.read()).toBe('classic');

    settingsPersistence.onboardingComplete.write(true);
    settingsPersistence.readerZoom.write(125);
    expect(storage.values.get(SETTINGS_STORAGE_KEYS.onboardingComplete)).toBe('true');
    expect(storage.values.get(SETTINGS_STORAGE_KEYS.readerZoom)).toBe('125');
  });

  it('survives a restart with the single-key shortcuts switched off', () => {
    const storage = memoryStorage();
    restoreProvider = setPersistenceStorageProvider(() => storage);

    // On by default: a fresh install is keyboard-first.
    expect(settingsPersistence.singleKeyShortcuts.read()).toBe(true);

    settingsPersistence.singleKeyShortcuts.write(false);

    expect(storage.values.get(SETTINGS_STORAGE_KEYS.singleKeyShortcuts)).toBe('false');
    expect(settingsPersistence.singleKeyShortcuts.read()).toBe(false);
  });

  it('uses the exact missing-value defaults for primitive settings', () => {
    restoreProvider = setPersistenceStorageProvider(() => memoryStorage());

    expect(settingsPersistence.onboardingComplete.read()).toBe(false);
    expect(settingsPersistence.autoDarkHtmlMails.read()).toBe(true);
    expect(settingsPersistence.preferPlainTextReading.read()).toBe(false);
    expect(settingsPersistence.readerZoom.read()).toBe(100);
    expect(settingsPersistence.composePlainText.read()).toBe(false);
    expect(settingsPersistence.collapseQuotedHistory.read()).toBe(true);
    expect(settingsPersistence.attachmentOpenBehavior.read()).toBe('preview');
    expect(settingsPersistence.inboxGroupingMode.read()).toBe('smart');
  });

  it('repairs keybindings and swipe actions while preserving their write shapes', () => {
    const storage = memoryStorage({
      [SETTINGS_STORAGE_KEYS.keybindings]: JSON.stringify({
        nextThread: '  arrowdown  ',
        openItem: '',
        unknown: 'ignored',
      }),
      [SETTINGS_STORAGE_KEYS.swipeActions]: JSON.stringify({
        rightShort: 'aside',
        rightLong: 'old-action',
        leftShort: null,
        leftLong: 'none',
      }),
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    const keybindings = settingsPersistence.keybindings.read();
    expect(keybindings.nextThread).toBe('arrowdown');
    expect(keybindings.openItem).toBe('l');
    expect(keybindings.unknown).toBeUndefined();
    expect(settingsPersistence.swipeActions.read()).toEqual({
      rightShort: 'aside',
      rightLong: 'done',
      leftShort: 'pin',
      leftLong: 'none',
    });

    settingsPersistence.swipeActions.write({
      rightShort: 'toggle-read',
      rightLong: 'done',
      leftShort: 'aside',
      leftLong: 'trash',
    });
    expect(storage.values.get(SETTINGS_STORAGE_KEYS.swipeActions)).toBe(
      '{"rightShort":"toggle-read","rightLong":"done","leftShort":"aside","leftLong":"trash"}',
    );
  });

  it('normalizes contact, sender, domain, and host legacy values', () => {
    const storage = memoryStorage({
      [SETTINGS_STORAGE_KEYS.pinnedContacts]: JSON.stringify([
        ' Person@Example.COM ',
        'not-an-address',
        42,
      ]),
      [SETTINGS_STORAGE_KEYS.inboxSenderBuckets]: JSON.stringify({
        ' Person@Example.COM ': 'personal',
        'bad@example.com': 'unsupported',
      }),
      [SETTINGS_STORAGE_KEYS.acceptedSenders]: JSON.stringify({
        ' Person@Example.COM ': true,
        'SECOND@example.com': true,
      }),
      [SETTINGS_STORAGE_KEYS.acceptedDomains]: JSON.stringify([' @Example.COM ', '']),
      [SETTINGS_STORAGE_KEYS.trustedLinkHosts]: JSON.stringify(['Shop.Example.COM']),
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(settingsPersistence.pinnedContacts.read()).toEqual(['person@example.com']);
    expect(settingsPersistence.inboxSenderBuckets.read()).toEqual({
      'person@example.com': 'personal',
    });
    expect(settingsPersistence.acceptedSenders.read()).toEqual({
      'person@example.com': true,
      'second@example.com': true,
    });
    expect(settingsPersistence.acceptedDomains.read()).toEqual({ 'example.com': true });
    expect(settingsPersistence.trustedLinkHosts.read()).toEqual({ 'example.com': true });

    storage.values.set(SETTINGS_STORAGE_KEYS.acceptedSenders, JSON.stringify([
      ' Legacy@Example.COM ',
      3,
    ]));
    expect(settingsPersistence.acceptedSenders.read()).toEqual({ 'legacy@example.com': true });
  });

  it('filters invalid dates and sender review entries', () => {
    const storage = memoryStorage({
      [SETTINGS_STORAGE_KEYS.senderReviewStartedAt]: 'not-a-date',
      [SETTINGS_STORAGE_KEYS.dismissedNewSenders]: JSON.stringify({
        ' Person@Example.COM ': '2026-07-11T09:00:00.000Z',
        'bad@example.com': 'not-a-date',
        'empty@example.com': 42,
      }),
    });
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(settingsPersistence.senderReviewStartedAt.read()).toBeNull();
    expect(settingsPersistence.dismissedNewSenders.read()).toEqual({
      'person@example.com': '2026-07-11T09:00:00.000Z',
    });
  });

  it('falls back for malformed complex values without rewriting storage', () => {
    const storage = memoryStorage(Object.fromEntries([
      SETTINGS_STORAGE_KEYS.keybindings,
      SETTINGS_STORAGE_KEYS.swipeActions,
      SETTINGS_STORAGE_KEYS.pinnedContacts,
      SETTINGS_STORAGE_KEYS.inboxSenderBuckets,
      SETTINGS_STORAGE_KEYS.acceptedSenders,
      SETTINGS_STORAGE_KEYS.acceptedDomains,
      SETTINGS_STORAGE_KEYS.trustedLinkHosts,
      SETTINGS_STORAGE_KEYS.dismissedNewSenders,
    ].map((key) => [key, '{'])));
    restoreProvider = setPersistenceStorageProvider(() => storage);

    expect(settingsPersistence.keybindings.read().nextThread).toBe('j');
    expect(settingsPersistence.swipeActions.read()).toEqual({
      rightShort: 'toggle-read',
      rightLong: 'done',
      leftShort: 'pin',
      leftLong: 'trash',
    });
    expect(settingsPersistence.pinnedContacts.read()).toEqual([]);
    expect(settingsPersistence.inboxSenderBuckets.read()).toEqual({});
    expect(settingsPersistence.acceptedSenders.read()).toEqual({});
    expect(settingsPersistence.acceptedDomains.read()).toEqual({});
    expect(settingsPersistence.trustedLinkHosts.read()).toEqual({});
    expect(settingsPersistence.dismissedNewSenders.read()).toEqual({});
    expect(storage.values.get(SETTINGS_STORAGE_KEYS.keybindings)).toBe('{');
  });

  it('returns fresh mutable defaults and tolerates unavailable or throwing storage', () => {
    restoreProvider = setPersistenceStorageProvider(() => undefined);
    const firstSwipe = settingsPersistence.swipeActions.read();
    firstSwipe.leftShort = 'none';
    const firstContacts = settingsPersistence.pinnedContacts.read();
    firstContacts.push('changed@example.com');

    expect(settingsPersistence.swipeActions.read().leftShort).toBe('pin');
    expect(settingsPersistence.pinnedContacts.read()).toEqual([]);
    expect(() => settingsPersistence.keybindings.write({ nextThread: 'n' })).not.toThrow();

    const throwingStorage: StorageLike = {
      getItem: () => {
        throw new Error('get denied');
      },
      setItem: () => {
        throw new Error('set denied');
      },
      removeItem: () => {
        throw new Error('remove denied');
      },
    };
    const restoreThrowing = setPersistenceStorageProvider(() => throwingStorage);
    expect(settingsPersistence.dismissedNewSenders.read()).toEqual({});
    expect(() => settingsPersistence.dismissedNewSenders.write({})).not.toThrow();
    restoreThrowing();
  });
});
