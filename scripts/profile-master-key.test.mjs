import { describe, expect, it, vi } from 'vitest';
import { existsSync, mkdtempSync, mkdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { copyProfileMasterKey, LOCAL_KEY_MARKER, profileKeyringAccount, removeBackupMasterKey } from './profile-master-key.mjs';

const vectors = JSON.parse(readFileSync(new URL('../tests/fixtures/profile-keyring-accounts.json', import.meta.url), 'utf8'));
const key = Buffer.alloc(32, 7).toString('base64');
const oldKey = Buffer.alloc(32, 9).toString('base64');

/** @param {number} status @returns {import('node:child_process').SpawnSyncReturns<string>} */
function commandResult(status, stdout = '', stderr = '') {
  return { status, stdout, stderr, pid: 0, output: [null, stdout, stderr], signal: null };
}

/**
 * @typedef {import('./profile-master-key.mjs').SecurityRunner} SecurityRunner
 * @typedef {{ source: string, destination: string, store: Map<string, string>, run: import('vitest').Mock<SecurityRunner> }} Fixture
 * @param {(fixture: Fixture) => void} test
 */
function fixture(test) {
  const root = mkdtempSync(join(tmpdir(), 'satchel-key-test-'));
  const source = join(root, 'stable');
  const destination = join(root, 'daily');
  mkdirSync(source);
  mkdirSync(destination);
  const store = new Map([[profileKeyringAccount(source), key]]);
  const run = vi.fn(/** @type {SecurityRunner} */ ((_command, args, options) => {
    if (args[0] === '-i') {
      const match = String(options.input).match(/^add-generic-password -U -s satchel -a (\S+) -w (\S+)\n$/);
      expect(match).not.toBeNull();
      store.set(match[1], match[2]);
      return commandResult(0);
    }
    const account = args[args.indexOf('-a') + 1];
    if (args[0] === 'delete-generic-password') {
      return commandResult(store.delete(account) ? 0 : 44);
    }
    return store.has(account) ? commandResult(0, `${store.get(account)}\n`) : commandResult(44);
  }));
  try { test({ source, destination, store, run }); } finally { rmSync(root, { recursive: true, force: true }); }
}

describe('profile master keys', () => {
  it('shares the portable and legacy key-file manifest with Rust', () => {
    const core = readFileSync(new URL('../crates/satchel-core/src/db/mod.rs', import.meta.url), 'utf8');
    const helper = readFileSync(new URL('./profile-master-key.mjs', import.meta.url), 'utf8');
    expect(core).toContain('&include!("master-key-files.json")');
    expect(helper).toContain('../crates/satchel-core/src/db/master-key-files.json');
    const files = JSON.parse(readFileSync(new URL('../crates/satchel-core/src/db/master-key-files.json', import.meta.url), 'utf8'));
    expect(files).toEqual(['pgp-master-key-v1', 'crypto/master.key']);
  });

  it.each(vectors)('matches the shared Rust account for $path', ({ path, account }) => {
    expect(profileKeyringAccount(path)).toBe(account);
  });

  it('imports a keychain-only profile, verifies it and removes a stale fallback', () => fixture(({ source, destination, store, run }) => {
    writeFileSync(join(destination, 'pgp-master-key-v1'), oldKey);
    mkdirSync(join(source, 'crypto'));
    writeFileSync(join(source, 'crypto', 'master.key'), oldKey);
    copyProfileMasterKey(source, destination, { run });
    expect(store.get(profileKeyringAccount(destination))).toBe(key);
    expect(existsSync(join(destination, 'pgp-master-key-v1'))).toBe(false);
    for (const [, args, options] of run.mock.calls) {
      expect(args.join(' ')).not.toContain(key);
      expect((Array.isArray(options.stdio) ? options.stdio.slice(1) : options.stdio)).toEqual(['pipe', 'pipe']);
    }
  }));

  it('preserves an automatic backup key when the Daily key is later replaced', () => fixture(({ source, destination, store, run }) => {
    copyProfileMasterKey(source, destination, { run, backup: true });
    store.set(profileKeyringAccount(source), oldKey);
    expect(store.get(readFileSync(join(destination, LOCAL_KEY_MARKER), 'utf8').trim())).toBe(key);
    expect(existsSync(join(destination, 'pgp-master-key-v1'))).toBe(false);
    removeBackupMasterKey(destination, { run });
    expect(store.has(profileKeyringAccount(destination))).toBe(false);
    expect(store.get(profileKeyringAccount(source))).toBe(oldKey);
  }));

  it('restores the snapshot key from a moved backup without using the shared entry', () => fixture(({ source, destination, store, run }) => {
    const backup = join(source, 'backup');
    const moved = join(source, 'moved-backup');
    copyProfileMasterKey(source, backup, { run, backup: true });
    renameSync(backup, moved);
    store.set(profileKeyringAccount(source), oldKey);
    store.set('pgp-master-key-v1', oldKey);
    copyProfileMasterKey(moved, destination, { run });
    expect(store.get(profileKeyringAccount(destination))).toBe(key);
    store.delete(profileKeyringAccount(backup));
    expect(() => copyProfileMasterKey(moved, destination, { run })).toThrow('Could not read the backup master key');
    expect(store.get(profileKeyringAccount(destination))).toBe(key);
  }));

  it('prefers a legacy file and copies it with owner-only permissions if Keychain is locked', () => fixture(({ source, destination, store }) => {
    writeFileSync(join(source, 'pgp-master-key-v1'), oldKey);
    writeFileSync(join(destination, 'pgp-master-key-v1'), key, { mode: 0o644 });
    copyProfileMasterKey(source, destination, { run: () => commandResult(1, '', key) });
    expect(readFileSync(join(destination, 'pgp-master-key-v1'), 'utf8')).toBe(oldKey);
    expect(statSync(join(destination, 'pgp-master-key-v1')).mode & 0o777).toBe(0o600);
    expect(store.get(profileKeyringAccount(source))).toBe(key);
  }));

  it('accepts the older crypto file only when there is no active keychain entry', () => fixture(({ source, destination, store, run }) => {
    store.clear();
    mkdirSync(join(source, 'crypto'));
    writeFileSync(join(source, 'crypto', 'master.key'), oldKey);
    copyProfileMasterKey(source, destination, { run });
    expect(store.get(profileKeyringAccount(destination))).toBe(oldKey);
  }));

  it('rejects malformed file keys without printing their contents', () => fixture(({ source, destination, run }) => {
    writeFileSync(join(source, 'pgp-master-key-v1'), 'invalid-key-material');
    expect(() => copyProfileMasterKey(source, destination, { run })).toThrow('Invalid master key. No database was replaced.');
    expect(run).not.toHaveBeenCalled();
  }));

  it('reads the old shared keychain entry when no profile entry exists', () => fixture(({ source, destination, store, run }) => {
    store.delete(profileKeyringAccount(source));
    store.set('pgp-master-key-v1', key);
    copyProfileMasterKey(source, destination, { run });
    expect(store.get(profileKeyringAccount(destination))).toBe(key);
  }));

  it('fails closed on missing keys, denied access, failed writes and mismatched read-back without leaking secrets', () => fixture(({ source, destination, store, run }) => {
    store.clear();
    expect(() => copyProfileMasterKey(source, destination, { run })).toThrow('No master key found');
    expect(() => copyProfileMasterKey(source, destination, { run: () => commandResult(1, '', key) })).toThrow('Could not read');
    store.set(profileKeyringAccount(source), key);
    /** @type {SecurityRunner} */
    const failingWrite = (command, args, options) => args[0] === '-i' ? commandResult(1, '', key) : run(command, args, options);
    expect(() => copyProfileMasterKey(source, destination, { run: failingWrite })).toThrow('Could not verify');
    /** @type {SecurityRunner} */
    const mismatch = (command, args, options) => args[0] === '-i' ? commandResult(0, '', key) : run(command, args, options);
    expect(() => copyProfileMasterKey(source, destination, { run: mismatch })).toThrow('Could not verify');
    expect(existsSync(join(destination, 'pgp-master-key-v1'))).toBe(false);
  }));

  it('refuses to prune a marker pointing at another profile', () => fixture(({ source, destination, run }) => {
    writeFileSync(join(destination, LOCAL_KEY_MARKER), profileKeyringAccount(source));
    expect(() => removeBackupMasterKey(destination, { run })).toThrow('Invalid backup');
    expect(run).not.toHaveBeenCalled();
  }));

  it('rejects a malformed snapshot marker before accessing Keychain', () => fixture(({ source, destination, run }) => {
    writeFileSync(join(source, LOCAL_KEY_MARKER), 'unrelated-account');
    expect(() => copyProfileMasterKey(source, destination, { run })).toThrow('Invalid backup');
    expect(run).not.toHaveBeenCalled();
  }));
});
