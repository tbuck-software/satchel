#!/usr/bin/env node
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  renameSync,
  rmSync,
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

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, '-');
}

function sqliteLiteral(value) {
  return `'${value.replace(/'/g, "''")}'`;
}

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

  const entries = readdirSync(dailyRoot).filter((entry) => entry !== 'backups');
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

// Make sure the target app is not holding an open SQLite connection while the
// database is replaced. The command is harmless if the app is not running.
spawnSync('osascript', ['-e', 'tell application "Satchel Daily" to quit'], {
  stdio: 'ignore',
});

mkdirSync(dailyRoot, { recursive: true });
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
