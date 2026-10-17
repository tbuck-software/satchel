import { nextTick } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useSettingsStore } from './settings';

function memoryStorage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
    clear: () => values.clear(),
  };
}

describe('settings store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.stubGlobal('window', {
      localStorage: memoryStorage(),
      matchMedia: () => ({ matches: false, addEventListener: () => {} }),
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('remembers the single-key switch across a restart', async () => {
    const settings = useSettingsStore();
    expect(settings.singleKeyShortcuts).toBe(true);

    settings.singleKeyShortcuts = false;
    await nextTick();
    expect(window.localStorage.getItem('satchel.singleKeyShortcuts')).toBe('false');

    // A fresh store on the same storage — this is what the next start sees.
    setActivePinia(createPinia());
    expect(useSettingsStore().singleKeyShortcuts).toBe(false);
  });

  it('persists accepted senders immediately using a normalized address', () => {
    const settings = useSettingsStore();

    settings.acceptSender('  Person@Example.COM ');

    expect(settings.acceptedSenders).toEqual({ 'person@example.com': true });
    expect(window.localStorage.getItem('satchel.acceptedSenders')).toBe('["person@example.com"]');
  });

  it('takes over existing senders and records when future review starts', () => {
    vi.setSystemTime(new Date('2026-07-11T10:00:00.000Z'));
    const settings = useSettingsStore();

    settings.completeSenderReviewSetup([' First@Example.com ', 'second@example.com']);

    expect(settings.acceptedSenders).toEqual({
      'first@example.com': true,
      'second@example.com': true,
    });
    expect(settings.senderReviewStartedAt).toBe('2026-07-11T10:00:00.000Z');
    expect(window.localStorage.getItem('satchel.senderReviewStartedAt')).toBe('2026-07-11T10:00:00.000Z');
  });

  it('persists accepted domains immediately using a normalized domain', () => {
    const settings = useSettingsStore();

    settings.acceptDomain(' @Strato.DE ');

    expect(settings.acceptedDomains).toEqual({ 'strato.de': true });
    expect(window.localStorage.getItem('satchel.acceptedDomains')).toBe('["strato.de"]');

    settings.forgetAcceptedDomain('strato.de');

    expect(settings.acceptedDomains).toEqual({});
    expect(window.localStorage.getItem('satchel.acceptedDomains')).toBe('[]');
  });

  it('trusts a link host across its registrable domain and persists it', () => {
    const settings = useSettingsStore();

    settings.trustLinkHost('Shop.Example.com');

    expect(settings.isTrustedLinkHost('shop.example.com')).toBe(true);
    expect(settings.isTrustedLinkHost('other.shop.example.com')).toBe(true);
    expect(settings.isTrustedLinkHost('example.org')).toBe(false);
    expect(window.localStorage.getItem('satchel.trustedLinkHosts')).toBe('["example.com"]');

    // Persisted hosts hydrate back into the same coverage.
    window.localStorage.setItem('satchel.trustedLinkHosts', '["www.trusted.io"]');
    setActivePinia(createPinia());
    const rehydrated = useSettingsStore();
    expect(rehydrated.isTrustedLinkHost('newsletter.trusted.io')).toBe(true);

    rehydrated.forgetTrustedLinkHost('trusted.io');
    expect(rehydrated.isTrustedLinkHost('trusted.io')).toBe(false);
  });

  it('persists the point through which a deleted new sender was handled', () => {
    const settings = useSettingsStore();

    settings.setDismissedNewSender(' Person@Example.com ', '2026-07-11T09:00:00.000Z');

    expect(settings.dismissedNewSenders).toEqual({
      'person@example.com': '2026-07-11T09:00:00.000Z',
    });
    expect(window.localStorage.getItem('satchel.dismissedNewSenders'))
      .toBe('{"person@example.com":"2026-07-11T09:00:00.000Z"}');
  });

  it('persists whether quoted history should be collapsed', async () => {
    const settings = useSettingsStore();
    expect(settings.collapseQuotedHistory).toBe(true);

    settings.collapseQuotedHistory = false;
    await nextTick();

    expect(window.localStorage.getItem('satchel.collapseQuotedHistory')).toBe('false');
  });

  it('starts with Spark-style swipe actions and persists custom slots', async () => {
    const settings = useSettingsStore();

    expect(settings.swipeActions).toEqual({
      rightShort: 'toggle-read',
      rightLong: 'done',
      leftShort: 'pin',
      leftLong: 'trash',
    });

    settings.setSwipeAction('leftShort', 'aside');
    await nextTick();

    expect(window.localStorage.getItem('satchel.swipeActions')).toBe(
      '{"rightShort":"toggle-read","rightLong":"done","leftShort":"aside","leftLong":"trash"}',
    );
  });

  it('keeps valid saved swipe slots and repairs invalid ones', () => {
    window.localStorage.setItem('satchel.swipeActions', JSON.stringify({
      rightShort: 'aside',
      rightLong: 'something-old',
      leftShort: null,
      leftLong: 'none',
    }));

    const settings = useSettingsStore();

    expect(settings.swipeActions).toEqual({
      rightShort: 'aside',
      rightLong: 'done',
      leftShort: 'pin',
      leftLong: 'none',
    });
  });
});
