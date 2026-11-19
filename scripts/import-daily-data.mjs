#!/usr/bin/env node
import {
  copyFileSync,
  cpSync,
  existsSync,
  fstatSync,
  mkdirSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
} from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { copyProfileMasterKey } from './profile-master-key.mjs';

const home = process.env.HOME;
if (!home) {
  console.error('HOME is not set.');
  process.exit(1);
}

const stableRoot = join(home, 'Library', 'Application Support', 'de.tbuck.satchel', 'satchel');
const dailyRoot = join(home, 'Library', 'Application Support', 'de.tbuck.satchel.daily', 'satchel');
const stableDb = join(stableRoot, 'satchel.db');
const dailyDb = join(dailyRoot, 'satchel.db');
const profileLockName = 'satchel.db.lock';
const profileLock = join(dailyRoot, profileLockName);
const underProfileLock = '--under-profile-lock';

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, '-');
}

/**
 * @param {string} value
 */
function sqliteLiteral(value) {
  return `'${value.replace(/'/g, "''")}'`;
}

/**
 * @param {string} command
 * @param {string[]} args
 * @param {import('node:child_process').SpawnSyncOptions} [options]
 */
function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    stdio: 'inherit',
    ...options,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function backupDailyData() {
  if (!existsSync(dailyRoot)) {
    return;
  }

  const entries = readdirSync(dailyRoot).filter((entry) => !['backups', profileLockName].includes(entry));
  if (entries.length === 0) {
    return;
  }

  const backupDir = join(dailyRoot, 'backups', `pre-import-${timestamp()}`);
  mkdirSync(backupDir, { recursive: true });
  if (existsSync(dailyDb)) copyProfileMasterKey(dailyRoot, backupDir, { backup: true });
  for (const entry of entries) {
    if (['satchel.db', 'satchel.db-wal', 'satchel.db-shm', 'pgp-master-key-v1', 'crypto'].includes(entry)) continue;
    cpSync(join(dailyRoot, entry), join(backupDir, entry), { recursive: true });
  }
  if (existsSync(dailyDb)) {
    run('sqlite3', [dailyDb, `.backup ${sqliteLiteral(join(backupDir, 'satchel.db'))}`]);
  }
  // Only copy legacy crypto data after preserving the active key separately.
  if (existsSync(join(dailyRoot, 'crypto'))) {
    cpSync(join(dailyRoot, 'crypto'), join(backupDir, 'crypto'), {
      recursive: true,
      filter: (source) => source !== join(dailyRoot, 'crypto', 'master.key'),
    });
  }
  console.log(`Backed up existing Daily data to ${backupDir}`);
  return backupDir;
}

/**
 * @param {string} source
 * @param {string} target
 */
function copyIfExists(source, target) {
  if (!existsSync(source)) {
    return;
  }
  mkdirSync(join(target, '..'), { recursive: true });
  copyFileSync(source, target);
}

if (process.platform !== 'darwin') {
  console.error('Daily data import is currently macOS-only.');
  process.exit(1);
}

if (!existsSync(stableDb)) {
  console.error(`Stable database not found: ${stableDb}`);
  process.exit(1);
}

if (process.argv[2] !== underProfileLock) {
  spawnSync('osascript', ['-e', 'tell application "Satchel Daily" to quit'], { stdio: 'ignore' });
  mkdirSync(dailyRoot, { recursive: true });

  // Rust uses BSD flock on this file on macOS. Keep its descriptor open
  // across exec so the import itself owns the lock through key and file swaps.
  run('/usr/bin/perl', ['-MFcntl=:DEFAULT,:flock', '-e', `
    $^F = 1024;
    my ($lock_path, @command) = @ARGV;
    sysopen(my $lock, $lock_path, O_RDWR | O_CREAT, 0600) or die "Open Daily profile lock: $!\n";
    my $locked = 0;
    for (1..20) {
      if (flock($lock, LOCK_EX | LOCK_NB)) { $locked = 1; last; }
      select(undef, undef, undef, 0.25);
    }
    die "The Daily profile is still in use. Quit Satchel Daily and retry.\n" unless $locked;
    exec(@command, fileno($lock)) or die "Start Daily import: $!\n";
  `, '--', profileLock, process.execPath, process.argv[1], underProfileLock]);
  process.exit(0);
}

// Check the inherited descriptor and reassert the same lock on it. flock
// belongs to the open file description shared by this process and the helper.
try {
  const fd = Number(process.argv[3]);
  if (!Number.isInteger(fd) || fd < 3) throw new Error('Missing Daily profile lock descriptor.');
  const openLock = fstatSync(fd, { bigint: true });
  const expected = statSync(profileLock, { bigint: true });
  if (openLock.dev !== expected.dev || openLock.ino !== expected.ino) {
    throw new Error('Invalid Daily profile lock descriptor.');
  }
  run('/usr/bin/perl', ['-MFcntl=:flock', '-e', `
    open(my $lock, '+<&=', 3) or die "Open inherited Daily profile lock: $!\n";
    flock($lock, LOCK_EX | LOCK_NB) or die "Acquire Daily profile lock: $!\n";
  `], { stdio: ['inherit', 'inherit', 'inherit', fd] });
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
if (['satchel.restore-ready.json', 'satchel.restore-applying.json'].some((name) => existsSync(join(dailyRoot, name)))) {
  console.error('Daily has a pending restore. Apply or cancel it before importing Stable data.');
  process.exit(1);
}

const previousDailyBackup = backupDailyData();

const tempDb = join(dailyRoot, `satchel.import-${timestamp()}.db`);
rmSync(tempDb, { force: true });
run('sqlite3', [stableDb, `.backup ${sqliteLiteral(tempDb)}`]);

// Verify the imported key before replacing the database.
try {
  copyProfileMasterKey(stableRoot, dailyRoot);
  for (const suffix of ['-wal', '-shm']) rmSync(`${dailyDb}${suffix}`, { force: true });
  renameSync(tempDb, dailyDb);
} catch (error) {
  if (previousDailyBackup && existsSync(join(previousDailyBackup, 'satchel.db'))) {
    try {
      copyFileSync(join(previousDailyBackup, 'satchel.db'), tempDb);
      copyProfileMasterKey(previousDailyBackup, dailyRoot);
      for (const suffix of ['-wal', '-shm']) rmSync(`${dailyDb}${suffix}`, { force: true });
      renameSync(tempDb, dailyDb);
    } catch {
      console.error(`Could not restore the previous Daily data. Recover from ${previousDailyBackup} through Backup & Export.`);
    }
  }
  rmSync(tempDb, { force: true });
  console.error(error.message);
  process.exit(1);
}

copyIfExists(join(stableRoot, '.env'), join(dailyRoot, '.env'));
if (existsSync(join(stableRoot, 'crypto'))) {
  rmSync(join(dailyRoot, 'crypto'), { recursive: true, force: true });
  cpSync(join(stableRoot, 'crypto'), join(dailyRoot, 'crypto'), {
    recursive: true,
    filter: (source) => source !== join(stableRoot, 'crypto', 'master.key'),
  });
}

console.log(`Imported Stable data into Daily: "${dailyDb}"`);
console.log('Copied the credential master key for the Daily profile.');
