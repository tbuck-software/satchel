import { describe, expect, it } from 'vitest';
import { migrationProblems } from './check-migrations.mjs';

describe('migration version guard', () => {
  const base = { '0022_old.sql': 'old', '0080_latest.sql': 'latest' };
  it('accepts additive migrations above the current main maximum', () => {
    expect(migrationProblems({ ...base, '0081_new.sql': 'new' }, base)).toEqual([]);
  });
  it('rejects two names with the same numeric version', () => {
    expect(migrationProblems({ ...base, '0081_a.sql': 'a', '81_b.sql': 'b' }, base).join()).toContain('used by');
  });
  it('rejects filling a retired number and collisions with main', () => {
    for (const name of ['0023_retired.sql', '0080_other.sql']) {
      expect(migrationProblems({ ...base, [name]: 'new' }, base).join()).toContain('must be above 80');
    }
  });
  it('rejects changing, removing or renaming already applied SQL', () => {
    for (const content of ['changed', 'old\n', ' old']) {
      expect(migrationProblems({ ...base, '0022_old.sql': content }, base).join()).toContain('modified');
    }
    expect(migrationProblems({ '0080_latest.sql': 'latest' }, base).join()).toContain('removed or renamed');
  });
  it('rejects malformed SQL filenames', () => {
    expect(() => migrationProblems({ 'latest.sql': 'new' }, base)).toThrow('Invalid migration filename');
  });
});
