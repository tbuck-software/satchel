import { beforeAll, describe, expect, it } from 'vitest';
import en from '@/i18n/en.json';
import {
  buildSettingsIndex,
  loadSettingsCatalogue,
  searchSettings,
  type SettingsSearchItem,
} from './settingsSearch';
import type { SettingsSection } from './settingsSections';

type Catalogue = Awaited<ReturnType<typeof loadSettingsCatalogue>>;
let parts: Catalogue;

function items(): SettingsSearchItem[] {
  const sections = en.settings.sections as unknown as Record<string, { label?: string; description?: string }>;
  return Object.entries(sections)
    .filter(([, entry]) => typeof entry.label === 'string')
    .map(([id, entry]) => ({ id: id as SettingsSection, label: entry.label!, description: entry.description ?? '' }));
}

beforeAll(async () => {
  parts = await loadSettingsCatalogue();
});

// #164: 26 sections and nothing to filter them by.
describe('settings search', () => {
  it('finds About by version or license information in both languages', () => {
    expect(searchSettings('version', items(), buildSettingsIndex(parts, 'en'))[0]?.id).toBe('about');
    expect(searchSettings('MIT-Lizenz', items(), buildSettingsIndex(parts, 'de')).map(result => result.id)).toContain('about');
  });
  it('finds a section by a setting inside it, and says which one', () => {
    const [first] = searchSettings('undo', items(), buildSettingsIndex(parts, 'en'));
    expect(first?.id).toBe('sending');
    expect(first?.hint).toMatch(/undo/i);
  });

  it('ranks a section named after the query above one that only mentions it', () => {
    const results = searchSettings('signature', items(), buildSettingsIndex(parts, 'en'));
    expect(results[0]?.id).toBe('signatures');
    expect(results[0]?.hint).toBeNull();
  });

  it('needs every word, in any order', () => {
    const results = searchSettings('sending delay', items(), buildSettingsIndex(parts, 'en'));
    expect(results.map((result) => result.id)).toContain('sending');
    expect(searchSettings('undo zzzz', items(), buildSettingsIndex(parts, 'en'))).toEqual([]);
  });

  it('searches the German strings in German', () => {
    const results = searchSettings('abwesenheit', items(), buildSettingsIndex(parts, 'de'));
    expect(results.map((result) => result.id)).toContain('vacation');
  });

  it('leaves outcome messages out of the index', () => {
    const terms = buildSettingsIndex(parts, 'en').get('accounts') ?? [];
    expect(terms.some((term) => term.startsWith('Delete failed'))).toBe(false);
    expect(terms).toContain('Test IMAP connection');
  });
});
