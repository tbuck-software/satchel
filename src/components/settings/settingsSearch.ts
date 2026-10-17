import { normalizeSearchText, rankSearchMatch, searchQuery } from '@/utils/searchScore';
import type { SettingsSection } from './settingsSections';

/**
 * Where each section's strings live in the i18n parts. A section is found by
 * what is written on it, not only by its name: "undo" lands on Sending.
 * Prefixes match the dotted key, so `settings.notifications.undo` covers
 * `undoLabel` and `undoHint` and leaves the rest of that shared table alone.
 */
const SECTION_KEYS: Record<SettingsSection, readonly string[]> = {
  general: ['settings.theme', 'settings.language'],
  swipes: ['settings.swipes', 'swipe.'],
  calendar: ['settings.calendar'],
  notifications: [
    'settings.notifications.title',
    'settings.notifications.description',
    'settings.notifications.enabled',
    'settings.notifications.scope',
    'settings.notifications.sound',
    'settings.notifications.permission',
  ],
  'mobile-sync': ['settings.mobileSync'],
  tray: ['settings.tray'],
  shortcuts: ['settings.shortcuts'],
  accounts: ['settings.accounts'],
  contacts: ['settings.contacts'],
  signatures: ['settings.signatures'],
  aliases: ['settings.aliases'],
  smime: ['settings.smime'],
  pgp: ['settings.pgp'],
  sending: [
    'settings.notifications.sending',
    'settings.notifications.undo',
    'settings.notifications.receipts',
  ],
  data: [
    'settings.notifications.data',
    'settings.notifications.bodyWindow',
    'settings.notifications.trash',
    'settings.inlineImages',
  ],
  vacation: ['settings.vacation'],
  blocked: ['settings.blocked'],
  images: ['settings.images'],
  'smart-folders': ['settings.smartFolders'],
  'provider-features': ['settings.providerFeatures'],
  backup: ['settings.backup'],
  sources: ['settings.sources'],
  templates: ['settings.templates'],
  rules: ['settings.rules'],
  plugins: ['settings.plugins', 'settings.api'],
  diagnostics: ['settings.diagnostics'],
};

// Outcome messages are not what anyone looks a setting up by.
const OUTCOME_KEY = /(?:Failed|Error|Confirm\w*|Saved|Deleted|Restored|Removed)$/;

type CataloguePart = Record<string, unknown>;

let catalogue: Promise<CataloguePart[]> | null = null;

/** The settings parts, loaded with the first search rather than with Settings. */
export function loadSettingsCatalogue(): Promise<CataloguePart[]> {
  catalogue ??= Promise.all([
    import('@/i18n/parts/settings-a.json'),
    import('@/i18n/parts/settings-b.json'),
    import('@/i18n/parts/settings-c.json'),
    import('@/i18n/parts/language.json'),
  ]).then((modules) => modules.map((module) => module.default as CataloguePart));
  return catalogue;
}

function collectStrings(node: unknown, path: string, out: Map<string, string>) {
  if (typeof node === 'string') {
    out.set(path, node);
    return;
  }
  if (!node || typeof node !== 'object') return;
  for (const [key, value] of Object.entries(node)) {
    collectStrings(value, path ? `${path}.${key}` : key, out);
  }
}

/** What each section says, in `locale`, with placeholders and plural forms flattened. */
export function buildSettingsIndex(
  parts: readonly CataloguePart[],
  locale: string,
): Map<SettingsSection, string[]> {
  const strings = new Map<string, string>();
  for (const part of parts) collectStrings(part[locale] ?? part.en, '', strings);
  const index = new Map<SettingsSection, string[]>();
  for (const [section, prefixes] of Object.entries(SECTION_KEYS) as [SettingsSection, readonly string[]][]) {
    const terms: string[] = [];
    for (const [key, text] of strings) {
      if (!prefixes.some((prefix) => key.startsWith(prefix)) || OUTCOME_KEY.test(key)) continue;
      for (const form of text.split('|')) {
        const plain = form.replace(/\{[^}]*\}/g, ' ').replace(/\s+/g, ' ').trim();
        if (plain) terms.push(plain);
      }
    }
    index.set(section, terms);
  }
  return index;
}

export interface SettingsSearchItem {
  id: SettingsSection;
  label: string;
  description: string;
}

export interface SettingsSearchResult extends SettingsSearchItem {
  /** The line inside the section that matched, when its name did not. */
  hint: string | null;
}

/** Sections answering the query, best first; ties keep sidebar order. */
export function searchSettings(
  raw: string,
  items: readonly SettingsSearchItem[],
  index: ReadonlyMap<SettingsSection, readonly string[]>,
): SettingsSearchResult[] {
  const query = searchQuery(raw);
  if (query.tokens.length === 0) return items.map((item) => ({ ...item, hint: null }));
  return items
    .flatMap((item, order) => {
      const terms = index.get(item.id) ?? [];
      // Name and description first; a hit only inside the section ranks below every such match.
      const named = rankSearchMatch(query, item.label, [item.description]);
      const rank = named !== null
        ? 10 + named
        : rankSearchMatch(query, item.label, [item.description, ...terms]);
      if (rank === null) return [];
      const label = normalizeSearchText(item.label);
      const hint = query.tokens.every((token) => label.includes(token))
        ? null
        : [item.description, ...terms].find((term) => normalizeSearchText(term).includes(query.phrase))
          ?? [item.description, ...terms].find((term) => normalizeSearchText(term).includes(query.tokens[0]!))
          ?? null;
      return [{ result: { ...item, hint }, rank, order }];
    })
    .sort((left, right) => right.rank - left.rank || left.order - right.order)
    .map(({ result }) => result);
}
