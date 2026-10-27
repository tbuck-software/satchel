import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { createI18n } from 'vue-i18n';
import { messageKeys, moduleGraph } from '../../scripts/i18n-graph.mjs';
import { SUBSETS, subsetKeys } from '../../scripts/i18n-merge.mjs';
import de from './de.json';
import en from './en.json';
import trayDe from './tray.de.json';
import trayEn from './tray.en.json';
import viewerDe from './viewer.de.json';
import viewerEn from './viewer.en.json';
import { localeFromLanguageTag, resolveLocale, systemLocale } from './locale';
import { KEYBINDING_SPECS } from '@/config/keybindingDefaults';
import { MAILBOX_SECTIONS } from '@/config/mailboxSections';
import { settingsSectionIcons } from '@/components/settings/settingsSections';

function keyPaths(value: unknown, trail: string[] = []): string[] {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return [trail.join('.')];
  return Object.entries(value as Record<string, unknown>)
    .flatMap(([key, child]) => keyPaths(child, [...trail, key]));
}

describe('message catalogue', () => {
  const enKeys = new Set(keyPaths(en));

  it('contains every statically referenced key', () => {
    const referenced = messageKeys(moduleGraph([
      'src/main.ts',
      'src/viewer/main.ts',
      'src/tray/main.ts',
    ]));
    expect([...referenced].filter((key) => !enKeys.has(key)).sort()).toEqual([]);
  });

  it('contains every key assembled dynamically at runtime', () => {
    const dynamicKeys = new Set<string>();

    for (const { action, view } of KEYBINDING_SPECS) {
      // A view's label and keywords are built from the sidebar's names.
      if (view) {
        for (const key of [view.labelKey, ...(view.aliasKeys ?? [])]) dynamicKeys.add(key);
        continue;
      }
      dynamicKeys.add(`shortcuts.actions.${action}.label`);
      dynamicKeys.add(`shortcuts.actions.${action}.keywords`);
    }

    for (const section of MAILBOX_SECTIONS) {
      dynamicKeys.add(section.labelKey);
      if (section.hintKey) dynamicKeys.add(section.hintKey);
    }

    for (const section of Object.keys(settingsSectionIcons)) {
      dynamicKeys.add(`settings.sections.${section}.label`);
      dynamicKeys.add(`settings.sections.${section}.description`);
    }

    [
      'thread.state.inbox',
      'thread.state.todo',
      'thread.state.waiting',
      'thread.state.aside',
      'thread.state.done',
      'thread.state.snoozed',
      'compose.title.new',
      'compose.title.resend',
      'outbox.status.failed',
      'outbox.status.sending',
      'outbox.status.scheduled',
      'outbox.status.pending',
      'outbox.action.sendMessage',
      'outbox.action.remoteMutation',
      'outbox.time.overdueSince',
      'outbox.time.scheduledFor',
      'outbox.time.nextRetry',
      'outbox.time.started',
      'outbox.time.queued',
    ].forEach((key) => dynamicKeys.add(key));

    expect([...dynamicKeys].filter((key) => !enKeys.has(key)).sort()).toEqual([]);
  });

  it('carries the same keys in both locales', () => {
    expect([...keyPaths(de)].sort()).toEqual([...enKeys].sort());
  });

  /**
   * The German copy says "du". The PGP and S/MIME texts had switched to
   * "Sie", so one dialog addressed the reader two ways.
   */
  it('addresses the reader as du throughout the German copy', () => {
    const lookup = (key: string) => key.split('.').reduce<unknown>(
      (node, segment) => (node as Record<string, unknown>)?.[segment],
      de,
    );
    const formal = /\b(Sie|Ihnen|Ihre[mnrs]?)\b/;
    const offenders = keyPaths(de).filter((key) => {
      const value = lookup(key);
      return typeof value === 'string' && formal.test(value);
    });
    expect(offenders).toEqual([]);
  });

  it('keeps placeholders identical across locales', () => {
    const placeholders = (bundle: unknown, key: string) => {
      const value = key.split('.').reduce<unknown>(
        (node, segment) => (node as Record<string, unknown>)?.[segment],
        bundle,
      );
      return typeof value === 'string'
        ? [...value.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort()
        : [];
    };
    const mismatched = [...enKeys].filter(
      (key) => placeholders(en, key).join() !== placeholders(de, key).join(),
    );
    expect(mismatched).toEqual([]);
  });

  it('compiles every message in both locales', () => {
    const instance = createI18n({
      legacy: false,
      locale: 'en',
      fallbackLocale: 'en',
      messages: { en, de },
    });

    for (const locale of ['en', 'de'] as const) {
      instance.global.locale.value = locale;
      for (const key of enKeys) {
        expect(() => instance.global.t(key)).not.toThrow();
      }
    }
  });

  /*
   * The viewer and the tray are separate bundles so those windows do not pay
   * for the client's catalogue. That only holds while their subsets stay
   * complete: a missing key surfaces as a raw key on screen, and nothing else
   * would notice.
   */
  it.each(Object.keys(SUBSETS))('%s subset covers every key that window reaches', (name) => {
    const spec = SUBSETS[name];
    if (!spec) throw new Error(`unknown i18n subset: ${name}`);
    const referenced = messageKeys(moduleGraph(spec.entries));
    const covered = new Set(subsetKeys(name, enKeys));
    const missing = [...referenced].filter((key) => !covered.has(key));
    expect(missing).toEqual([]);
  });

  it('ships the generated subsets', () => {
    expect(keyPaths(viewerEn).sort()).toEqual(keyPaths(viewerDe).sort());
    expect(keyPaths(trayEn).sort()).toEqual(keyPaths(trayDe).sort());
    for (const key of keyPaths(viewerEn)) expect(enKeys.has(key)).toBe(true);
    for (const key of keyPaths(trayEn)) expect(enKeys.has(key)).toBe(true);
  });

  it('includes the viewer keys assembled dynamically at runtime', () => {
    const viewerKeys = new Set(keyPaths(viewerEn));
    // `g` destinations are named after the sidebar; the viewer has none.
    for (const { action } of KEYBINDING_SPECS.filter((spec) => !spec.view)) {
      expect(viewerKeys.has(`shortcuts.actions.${action}.label`)).toBe(true);
      expect(viewerKeys.has(`shortcuts.actions.${action}.keywords`)).toBe(true);
    }
    expect(viewerKeys.has('compose.title.new')).toBe(true);
    expect(viewerKeys.has('compose.title.resend')).toBe(true);
  });

  /*
   * Same contract for the viewer: `commands/message_file.rs` picks the keys,
   * and a key the subset misses would reach the EmptyState raw.
   */
  it('carries every backend error the message file commands emit', () => {
    const viewerKeys = new Set(keyPaths(viewerEn));
    const source = ['message_file', 'reveal'].map((name) =>
      readFileSync(`src-tauri/src/commands/${name}.rs`, 'utf8')).join('\n');
    const emitted = new Set(source.match(/backend\.errors\.[A-Za-z0-9_]+/g) ?? []);

    expect(viewerKeys.has('backend.errors.unexpected')).toBe(true);
    expect([...emitted].filter((key) => !viewerKeys.has(key)).sort()).toEqual([]);
    expect(viewerKeys.has('backend.errors.webhookUrlInvalid')).toBe(false);
  });

  /*
   * Backend errors reach the tray as keys chosen by `commands/tray.rs`. The
   * subset carries those and the generic fallback; the client's other 140-odd
   * error strings once made up most of the tray entry bundle.
   */
  it('limits the tray subset to the errors its commands emit', () => {
    const trayKeys = new Set(keyPaths(trayEn));
    expect(trayKeys.has('backend.errors.trayThreadInvalid')).toBe(true);
    expect(trayKeys.has('backend.errors.trayDestinationInvalid')).toBe(true);
    expect(trayKeys.has('backend.errors.trayMessageNotFound')).toBe(true);
    expect(trayKeys.has('backend.errors.unexpected')).toBe(true);
    expect(trayKeys.has('backend.errors.webhookUrlInvalid')).toBe(false);
    expect([...trayKeys].filter((key) => key.startsWith('backend.errors.')).length).toBeLessThan(12);
  });
});

describe('locale resolution', () => {
  it('maps a language tag to a shipped locale', () => {
    expect(localeFromLanguageTag('de-AT')).toBe('de');
    expect(localeFromLanguageTag('en-GB')).toBe('en');
    expect(localeFromLanguageTag('fr-FR')).toBeNull();
  });

  it('falls back to English for languages we do not ship', () => {
    expect(systemLocale(['fr-FR', 'it'])).toBe('en');
    expect(systemLocale(['fr-FR', 'de-CH'])).toBe('de');
  });

  it('honours an explicit preference over the system languages', () => {
    expect(resolveLocale('de', ['en-US'])).toBe('de');
    expect(resolveLocale('system', ['de-DE'])).toBe('de');
  });
});
