import { describe, expect, it } from 'vitest';
import { normalizeSearchText, rankByQuery, rankSearchMatch, searchQuery } from './searchScore';

describe('rankSearchMatch', () => {
  it('matches words in any order and across title and terms', () => {
    expect(rankSearchMatch(searchQuery('sig edit'), 'Edit signatures')).toBe(2);
    expect(rankSearchMatch(searchQuery('undo send'), 'Sending', ['Undo window'])).not.toBeNull();
    expect(rankSearchMatch(searchQuery('undo archive'), 'Sending', ['Undo window'])).toBeNull();
  });

  it('puts an exact or leading title match above a match in the terms', () => {
    const query = searchQuery('sign');
    const ranks = [
      rankSearchMatch(query, 'Signatures'),
      rankSearchMatch(query, 'Edit signatures'),
      rankSearchMatch(query, 'Accounts', ['Check or renew sign-in']),
    ];
    expect(ranks[0]!).toBeGreaterThan(ranks[1]!);
    expect(ranks[1]!).toBeGreaterThan(ranks[2]!);
  });

  it('folds case and accents', () => {
    expect(normalizeSearchText('  Über  Ärger ')).toBe('uber arger');
    expect(rankSearchMatch(searchQuery('uber'), 'Über Satchel')).toBe(5);
  });
});

// #164: the palette matched the whole query as one substring, unranked.
describe('rankByQuery', () => {
  const commands = [
    { id: 'theme', title: 'Toggle theme', keywords: 'dark light appearance' },
    { id: 'settings:signatures', title: 'Settings: Signatures', keywords: 'Signatures for your identities settings' },
    { id: 'settings:shortcuts', title: 'Edit keyboard shortcuts', keywords: 'keybindings shortcuts customize' },
    { id: 'settings:sending', title: 'Settings: Sending', keywords: 'Delivery, undo send and confirmations settings' },
  ];

  it('finds a command by words in any order and puts title matches first', () => {
    expect(rankByQuery(commands, searchQuery('signatures settings')).map((entry) => entry.id))
      .toEqual(['settings:signatures']);
    expect(rankByQuery(commands, searchQuery('settings')).map((entry) => entry.id)[0])
      .toBe('settings:signatures');
  });

  it('breaks ties by recent use', () => {
    const ids = rankByQuery(commands, searchQuery('settings'), ['settings:sending']).map((entry) => entry.id);
    expect(ids.indexOf('settings:sending')).toBeLessThan(ids.indexOf('settings:signatures'));
  });
});
