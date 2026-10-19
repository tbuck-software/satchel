import { createHash, randomUUID } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import keyFiles from '../crates/satchel-core/src/db/master-key-files.json' with { type: 'json' };

const SERVICE = 'satchel';
const LEGACY_ACCOUNT = 'pgp-master-key-v1';
/** @type {string[]} */
const KEY_FILES = keyFiles;
export const LOCAL_KEY_MARKER = 'master-key.keychain';

/** @typedef {(command: string, args: string[], options: import('node:child_process').SpawnSyncOptionsWithStringEncoding) => { status: number | null, stdout: string }} SecurityRunner */

// Rust hashes the unnormalized native path bytes: UTF-8 on macOS, UTF-16LE on Windows.
/**
 * @param {string} profilePath
 */
export function profileKeyringAccount(profilePath) {
  return `${LEGACY_ACCOUNT}:${createHash('sha256').update(Buffer.from(profilePath, 'utf8')).digest('hex')}`;
}

/** @param {string} account @param {SecurityRunner} run */
function readAccount(account, run) {
  const result = run('/usr/bin/security', ['find-generic-password', '-s', SERVICE, '-a', account, '-w'], {
    encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
  });
  if (result.status === 44) return undefined; // errSecItemNotFound
  if (result.status !== 0) throw new Error('Could not read the master key from Keychain. No database was replaced.');
  return validateKey(result.stdout.trim());
}

/**
 * @param {string} key
 */
function validateKey(key) {
  if (
    !/^[A-Za-z0-9+/]{43}=$/.test(key)
    || Buffer.from(key, 'base64').length !== 32
    || Buffer.from(key, 'base64').toString('base64') !== key
  ) {
    throw new Error('Invalid master key. No database was replaced.');
  }
  return key;
}

/** @param {string} account @param {string} key @param {SecurityRunner} run */
function writeAccount(account, key, run) {
  // Interactive mode reads commands from stdin. Capture both outputs: security
  // may echo its input on errors. The key never enters argv or our error text.
  const result = run('/usr/bin/security', ['-i'], {
    input: `add-generic-password -U -s ${SERVICE} -a ${account} -w ${key}\n`,
    encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'],
  });
  if (result.status !== 0 || readAccount(account, run) !== key) {
    throw new Error('Could not verify the copied master key in Keychain. No database was replaced.');
  }
}

/**
 * @param {string} directory
 */
function backupAccount(directory) {
  const marker = join(directory, LOCAL_KEY_MARKER);
  if (!existsSync(marker)) return undefined;
  const account = readFileSync(marker, 'utf8').trim();
  if (!/^pgp-master-key-v1:[a-f0-9]{64}$/.test(account)) {
    throw new Error('Invalid backup keychain marker.');
  }
  return account;
}

// Automatic backups keep a separate keychain item, with legacy files as a fallback.
/** @param {string} source @param {string} destination @param {{ backup?: boolean, run?: SecurityRunner }} [options] */
export function copyProfileMasterKey(source, destination, { backup = false, run = spawnSync } = {}) {
  let legacy = existsSync(join(source, KEY_FILES[0])) ? KEY_FILES[0] : undefined;
  let key;
  if (legacy) {
    key = validateKey(readFileSync(join(source, legacy), 'utf8').trim());
  } else {
    const snapshot = backupAccount(source);
    key = snapshot
      ? readAccount(snapshot, run)
      : readAccount(profileKeyringAccount(source), run) ?? readAccount(LEGACY_ACCOUNT, run);
    if (snapshot && !key) throw new Error('Could not read the backup master key from Keychain. No database was replaced.');
    // The older crypto file must never override an active native keychain key.
    if (!key && existsSync(join(source, KEY_FILES[1]))) {
      legacy = KEY_FILES[1];
      key = validateKey(readFileSync(join(source, legacy), 'utf8').trim());
    }
  }
  if (!key) throw new Error('No master key found. No database was replaced.');

  mkdirSync(destination, { recursive: true });
  const account = profileKeyringAccount(destination);
  try {
    writeAccount(account, key, run);
  } catch (error) {
    if (!legacy) throw error;
    // Preserve old profiles when Keychain is unavailable, as the Rust loader does.
    const target = join(destination, KEY_FILES[0]);
    const temporary = `${target}.${randomUUID()}`;
    try {
      writeFileSync(temporary, key, { mode: 0o600, flag: 'wx' });
      renameSync(temporary, target);
    } finally {
      rmSync(temporary, { force: true });
    }
    rmSync(join(destination, LOCAL_KEY_MARKER), { force: true });
    return;
  }
  if (backup) {
    writeFileSync(join(destination, LOCAL_KEY_MARKER), `${account}\n`, { mode: 0o600 });
  } else {
    // Rust prefers a fallback file over Keychain, so remove a stale Daily key.
    for (const name of KEY_FILES) rmSync(join(destination, name), { force: true });
  }
}

/** @param {string} directory @param {{ run?: SecurityRunner }} [options] */
export function removeBackupMasterKey(directory, { run = spawnSync } = {}) {
  const account = backupAccount(directory);
  if (!account) return;
  // Pruning must never delete another profile's key, even with a forged marker.
  if (account !== profileKeyringAccount(directory)) throw new Error('Invalid backup keychain marker.');
  const result = run('/usr/bin/security', ['delete-generic-password', '-s', SERVICE, '-a', account], {
    encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
  });
  if (result.status !== 0 && result.status !== 44) throw new Error('Could not remove the old backup key from Keychain.');
}
