import { describe, expect, it } from 'vitest';
import { requireReleaseTag, reserveReleaseTags } from './release-tags.mjs';

const source = 'a'.repeat(40);
const other = 'b'.repeat(40);

/** @param {Record<string, string>} [initial] */
function registry(initial = {}) {
  const values = new Map(Object.entries(initial));
  /** @type {{tag: string, source: string}[]} */
  const created = [];
  /** @type {{read: (tag: string) => string | null, create: (tag: string, source: string) => void}} */
  const tags = {
    read: tag => values.get(tag) ?? null,
    create(tag, sha) {
      if (values.has(tag)) throw new Error('A tag cannot be overwritten');
      created.push({ tag, source: sha });
      values.set(tag, sha);
    },
  };
  return { tags, created, values };
}

describe('immutable release source reservations', () => {
  it('reserves a new stable version at its verified source', () => {
    const repo = registry();
    expect(reserveReleaseTags({ channel: 'stable', version: '0.1.2', source }, repo.tags))
      .toEqual({ version: '0.1.2', tag: 'v0.1.2' });
    expect(repo.created).toEqual([{ tag: 'v0.1.2', source }]);
  });

  it('reuses a failed build reservation only for the same commit', () => {
    const repo = registry({ 'v0.1.2': source });
    expect(reserveReleaseTags({ channel: 'stable', version: '0.1.2', source }, repo.tags).tag).toBe('v0.1.2');
    expect(repo.created).toEqual([]);
  });

  it('skips reserved stable versions rather than moving another source tag', () => {
    const repo = registry({ 'v0.1.2': other, 'v0.1.3': other });
    expect(reserveReleaseTags({ channel: 'stable', version: '0.1.2', source }, repo.tags))
      .toEqual({ version: '0.1.4', tag: 'v0.1.4' });
    expect(repo.values.get('v0.1.2')).toBe(other);
    expect(repo.values.get('v0.1.3')).toBe(other);
    expect(repo.created).toEqual([{ tag: 'v0.1.4', source }]);
  });

  it('reserves the versioned nightly and the initial channel tag', () => {
    const repo = registry();
    reserveReleaseTags({ channel: 'nightly', version: '0.1.2-nightly.42', source }, repo.tags);
    expect(repo.created).toEqual([{ tag: 'v0.1.2-nightly.42', source }, { tag: 'nightly', source }]);
  });

  it('retains the existing rolling channel tag while reserving a new nightly', () => {
    const repo = registry({ nightly: other });
    reserveReleaseTags({ channel: 'nightly', version: '0.1.2-nightly.42', source }, repo.tags);
    expect(repo.values.get('nightly')).toBe(other);
    expect(repo.created).toEqual([{ tag: 'v0.1.2-nightly.42', source }]);
  });

  it('allows a rerun of the same nightly without rewriting tags', () => {
    const repo = registry({ 'v0.1.2-nightly.42': source, nightly: other });
    reserveReleaseTags({ channel: 'nightly', version: '0.1.2-nightly.42', source }, repo.tags);
    expect(repo.created).toEqual([]);
  });

  it('refuses a nightly run number already reserved by another commit', () => {
    const repo = registry({ 'v0.1.2-nightly.42': other });
    expect(() => reserveReleaseTags({ channel: 'nightly', version: '0.1.2-nightly.42', source }, repo.tags))
      .toThrow('another commit');
    expect(repo.created).toEqual([]);
  });

  it('requires the published tag to resolve to exactly the verified build source', () => {
    const repo = registry({ 'v0.1.2': source, 'v0.1.3': other });
    expect(() => requireReleaseTag('v0.1.2', source, repo.tags)).not.toThrow();
    expect(() => requireReleaseTag('v0.1.3', source, repo.tags)).toThrow('verified build');
    expect(() => requireReleaseTag('v0.1.4', source, repo.tags)).toThrow('verified build');
  });
});
