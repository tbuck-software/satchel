import { normalizeSearchText, rankSearchMatch, searchQuery } from '@/utils/searchScore';
import type { SettingsSection } from './settingsSections';

/**
 * Where each section's strings live in the i18n parts. A section is found by
 * what is written on it, not only by its name: "undo" lands on Sending.
 * Prefixes match the dotted key, so `settings.notifications.undo` covers
 * `undoLabel` and `undoHint` and leaves the rest of that shared table alone.
 */
const SECTION_KEYS: Record<SettingsSection, readonly string[]> = {
  about: ['settings.about', 'app.update'],
  general: ['settings.theme', 'settings.language', 'settings.tray'],
  swipes: ['settings.swipes', 'swipe.'],
  calendar: ['settings.calendar'],
  notifications: [
    'settings.notifications.title',
    'settings.notifications.description',
    'settings.notifications.enabled',
    'settings.notifications.scope',
    'settings.notifications.sound',
    'settings.notifications.permission',
    'settings.mobileSync',
  ],
  shortcuts: ['settings.shortcuts'],
  accounts: ['settings.accounts', 'settings.sources', 'settings.providerFeatures', 'settings.plugins.webhook'],
  contacts: ['settings.contacts'],
  signatures: ['settings.signatures'],
  aliases: ['settings.aliases'],
  encryption: ['settings.smime', 'settings.pgp'],
  translation: ['settings.translation'],
  composing: [
    'settings.notifications.sending',
    'settings.notifications.undo',
    'settings.notifications.receipts',
    'settings.theme.plainTextCompose',
  ],
  storage: [
    'settings.notifications.data',
    'settings.notifications.bodyWindow',
    'settings.notifications.trash',
    'settings.inlineImages',
    'settings.backup',
  ],
  vacation: ['settings.vacation'],
  senders: ['settings.spamFilter', 'settings.blocked', 'settings.images'],
  'smart-folders': ['settings.smartFolders'],
  templates: ['settings.templates'],
  rules: ['settings.rules', 'settings.sweep'],
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
    import('@/i18n/parts/translation.json'),
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
  // A short word only counts where a word starts: "sign" means signatures,
  // not "designs" or "assignment". Longer words may sit inside compounds
  // ("bestätigung" in "Lesebestätigung").
  const startsWord = (text: string, token: string) =>
    new RegExp(`(?:^|[^\\p{L}\\p{N}])${token.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`, 'u').test(text);
  const fair = (term: string) => {
    const text = normalizeSearchText(term);
    return query.tokens.every((token) => token.length >= 5 || !text.includes(token) || startsWord(text, token));
  };
  return items
    .flatMap((item, order) => {
      const terms = (index.get(item.id) ?? []).filter(fair);
      // Name and description first; a hit only inside the section ranks below every such match.
      const named = rankSearchMatch(query, item.label, [item.description].filter(fair));
      const rank = named !== null
        ? 10 + named
        : rankSearchMatch(query, item.label, [item.description, ...terms].filter(fair));
      if (rank === null) return [];
      const label = normalizeSearchText(item.label);
      const hint = query.tokens.every((token) => label.includes(token))
        ? null
        : [item.description, ...terms].filter(fair).find((term) => normalizeSearchText(term).includes(query.phrase))
          ?? [item.description, ...terms].filter(fair).find((term) => normalizeSearchText(term).includes(query.tokens[0]!))
          ?? null;
      return [{ result: { ...item, hint }, rank, order }];
    })
    .sort((left, right) => right.rank - left.rank || left.order - right.order)
    .map(({ result }) => result);
}
