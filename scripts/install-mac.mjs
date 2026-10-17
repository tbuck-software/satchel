#!/usr/bin/env node
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  utimesSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { processListContainsApp, stableInstallProblem, versionTagProblem } from './install-mac-helpers.mjs';
import { copyProfileMasterKey, removeBackupMasterKey } from './profile-master-key.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

const channels = {
  stable: {
    buildScript: 'tauri:install',
    appName: 'Satchel',
    mainBinary: 'satchel',
    identifier: 'de.tbuck.satchel',
    targetApp: '/Applications/Satchel.app',
  },
  daily: {
    buildScript: 'tauri:daily',
    appName: 'Satchel Daily',
    mainBinary: 'satchel',
    identifier: 'de.tbuck.satchel.daily',
    targetApp: '/Applications/Satchel Daily.app',
  },
};

function argValue(name, fallback) {
  const index = process.argv.indexOf(name);
  if (index < 0) return fallback;
  return process.argv[index + 1] ?? fallback;
}

const channelName = argValue('--channel', 'stable');
const shouldOpen = process.argv.includes('--open');
const channel = channels[channelName];
if (!channel) {
  console.error(`Unknown install channel: ${channelName}`);
  console.error(`Expected one of: ${Object.keys(channels).join(', ')}`);
  process.exit(1);
}

function gitOutput(args) {
  const result = spawnSync('git', args, { cwd: repoRoot, encoding: 'utf8' });
  if (result.status !== 0) {
    console.error(result.stderr || 'Failed to inspect the checkout before installing.');
    process.exit(1);
  }
  return result.stdout.trim();
}

const installRevision = gitOutput(['rev-parse', 'HEAD']);
const installStatus = gitOutput(['status', '--porcelain']);
const installProblem = stableInstallProblem({
  channel: channelName,
  branch: gitOutput(['rev-parse', '--abbrev-ref', 'HEAD']),
  status: installStatus,
  allowUnstable: process.argv.includes('--allow-unstable-stable-install'),
});
if (installProblem) {
  console.error(installProblem);
  process.exit(1);
}

const releaseTag = channelName === 'stable' && !installStatus && !process.argv.includes('--allow-unstable-stable-install')
  ? `v${JSON.parse(readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8')).version}`
  : null;
const existingTag = releaseTag && spawnSync('git', ['rev-parse', '--verify', `refs/tags/${releaseTag}^{commit}`], {
  cwd: repoRoot, encoding: 'utf8',
});
if (existingTag && existingTag.status !== 0 && existingTag.status !== 128) {
  console.error(existingTag.stderr || 'Failed to inspect the release tag.');
  process.exit(1);
}
const tagProblem = releaseTag && versionTagProblem(releaseTag,
  existingTag.status === 0 ? existingTag.stdout.trim() : '', installRevision);
if (tagProblem) {
  console.error(tagProblem);
  process.exit(1);
}

const appSupportRoot = join(
  process.env.HOME ?? '',
  'Library',
  'Application Support',
  channel.identifier,
  'satchel',
);
const appSupportEnv = join(appSupportRoot, '.env');

const builtAppCandidates = [
  join(repoRoot, 'target', 'release', 'bundle', 'macos', `${channel.appName}.app`),
  join(repoRoot, 'src-tauri', 'target', 'release', 'bundle', 'macos', `${channel.appName}.app`),
];

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    ...options,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, '-');
}

function shellQuote(value) {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function isAppRunning() {
  const result = spawnSync('ps', ['ax', '-o', 'command='], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'ignore'],
  });
  return (
    result.status === 0
    && processListContainsApp(result.stdout, channel.targetApp, channel.mainBinary)
  );
}

function quitRunningApp() {
  if (!isAppRunning()) {
    return;
  }

  console.log(`Quitting running ${channel.appName} before install...`);
  spawnSync('osascript', ['-e', `tell application id "${channel.identifier}" to quit`], {
    stdio: 'ignore',
  });

  for (let attempt = 0; attempt < 50; attempt += 1) {
    if (!isAppRunning()) {
      return;
    }
    sleepSync(200);
  }

  console.error(
    `${channel.appName} is still running. Quit it manually and rerun: ${
      channelName === 'daily' ? 'pnpm run install:daily:mac' : 'pnpm run install:mac'
    }`,
  );
  process.exit(1);
}

function bundleExecutable(appPath) {
  const result = spawnSync(
    'plutil',
    ['-extract', 'CFBundleExecutable', 'raw', '-o', '-', join(appPath, 'Contents', 'Info.plist')],
    { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
  );
  return result.status === 0 ? result.stdout.trim() : '';
}

function validateBuiltApp(appPath) {
  const executable = bundleExecutable(appPath);
  const binaryPath = join(appPath, 'Contents', 'MacOS', channel.mainBinary);
  if (executable !== channel.mainBinary || !existsSync(binaryPath)) {
    console.error(
      `Built app contains the wrong executable: expected ${channel.mainBinary}, found ${executable || 'none'}.`,
    );
    console.error(`Expected binary at ${binaryPath}`);
    process.exit(1);
  }
}

function openInstalledApp() {
  run('open', [channel.targetApp]);
  for (let attempt = 0; attempt < 50; attempt += 1) {
    if (isAppRunning()) {
      sleepSync(500);
      if (isAppRunning()) {
        console.log(`Opened ${channel.appName}.`);
        return;
      }
    }
    sleepSync(100);
  }
  console.error(`${channel.appName} was installed but did not remain running after launch.`);
  console.error(`Try: ${join(channel.targetApp, 'Contents', 'MacOS', channel.mainBinary)}`);
  process.exit(1);
}

/** How many pre-install/pre-import backups to keep. */
const BACKUP_KEEP = 3;

/**
 * Delete all but the newest `BACKUP_KEEP` backups.
 *
 * Without this every install left a full copy behind forever: 128 of them had
 * accumulated to ~370 GB on a disk that was 91% full.
 */
function pruneOldBackups() {
  const backupsRoot = join(appSupportRoot, 'backups');
  if (!existsSync(backupsRoot)) return;

  // Sort by the embedded timestamp, not the whole name: "pre-import-…" sorts
  // before "pre-install-…" lexically, so a plain sort would rank a newer
  // import backup below an older install one and delete the wrong files.
  const timestampOf = (name) => name.replace(/^pre-(install|import)-/, '');
  const entries = readdirSync(backupsRoot)
    .filter((name) => name.startsWith('pre-install-') || name.startsWith('pre-import-'))
    .sort((a, b) => timestampOf(b).localeCompare(timestampOf(a)));

  for (const stale of entries.slice(BACKUP_KEEP)) {
    try {
      removeBackupMasterKey(join(backupsRoot, stale));
      rmSync(join(backupsRoot, stale), { recursive: true, force: true });
      console.log(`Removed old backup ${stale}`);
    } catch (error) {
      console.warn(`Could not remove old backup ${stale}: ${error.message}`);
    }
  }
}

function backupAppData() {
  const dbPath = join(appSupportRoot, 'satchel.db');
  if (!existsSync(dbPath)) {
    return;
  }

  const backupDir = join(appSupportRoot, 'backups', `pre-install-${timestamp()}`);
  mkdirSync(backupDir, { recursive: true });

  // Use SQLite's backup API rather than copying db + wal + shm.
  //
  // A raw copy of a live database can catch a torn write, and it duplicated a
  // multi-gigabyte WAL alongside the database — that WAL was most of the
  // backup bloat. `.backup` folds the WAL in and writes one consistent file.
  const target = join(backupDir, 'satchel.db');
  const result = spawnSync('sqlite3', [dbPath, `.backup '${target.replace(/'/g, "''")}'`], {
    stdio: 'inherit',
  });

  if (result.status !== 0) {
    console.error('sqlite3 .backup failed; installation stopped before replacing the app.');
    process.exit(result.status ?? 1);
  }

  copyProfileMasterKey(appSupportRoot, backupDir, { backup: true });

  console.log(`Backed up ${channel.appName} data to ${backupDir}`);
  pruneOldBackups();
}

if (process.platform !== 'darwin') {
  console.error('install:mac is only supported on macOS.');
  process.exit(1);
}

quitRunningApp();
backupAppData();

// Tauri embeds the frontend (dist/) into the Rust binary at *compile* time.
// A frontend-only change leaves every Rust source untouched, so cargo reuses
// the previous binary and the freshly built dist/ never reaches the app —
// the install then silently ships the old UI. Touch the crate root so the
// embed always reruns.
const embedTouchTargets = [
  join(repoRoot, 'src-tauri', 'src', 'lib.rs'),
  join(repoRoot, 'src-tauri', 'src', 'main.rs'),
].filter((file) => existsSync(file));
const now = new Date();
for (const file of embedTouchTargets) {
  utimesSync(file, now, now);
}

run('pnpm', ['run', channel.buildScript]);
if (releaseTag && (gitOutput(['rev-parse', 'HEAD']) !== installRevision || gitOutput(['status', '--porcelain']))) {
  console.error('The checkout changed during the build. Rerun from the intended clean commit.');
  process.exit(1);
}
quitRunningApp();

const builtApp = builtAppCandidates.find((candidate) => existsSync(candidate));
if (!builtApp) {
  console.error(`Build finished but app bundle was not found at:\n${builtAppCandidates.join('\n')}`);
  process.exit(1);
}
validateBuiltApp(builtApp);

const stagedApp = `${channel.targetApp}.installing-${process.pid}`;
const previousApp = `${channel.targetApp}.previous-${process.pid}`;
rmSync(stagedApp, { recursive: true, force: true });
rmSync(previousApp, { recursive: true, force: true });
cpSync(builtApp, stagedApp, { recursive: true });

run('xattr', ['-dr', 'com.apple.quarantine', stagedApp]);

// Rust release binaries are linker-signed, but that does not seal the app
// bundle's Info.plist and resources. Ad-hoc sign the complete local bundle so
// LaunchServices never receives the incomplete bundle produced by the build.
run('codesign', ['--force', '--deep', '--sign', '-', stagedApp]);
run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', stagedApp]);

try {
  if (existsSync(channel.targetApp)) {
    renameSync(channel.targetApp, previousApp);
  }
  renameSync(stagedApp, channel.targetApp);
  rmSync(previousApp, { recursive: true, force: true });
} catch (error) {
  if (!existsSync(channel.targetApp) && existsSync(previousApp)) {
    renameSync(previousApp, channel.targetApp);
  }
  rmSync(stagedApp, { recursive: true, force: true });
  throw error;
}

if (process.env.HOME && existsSync(join(repoRoot, '.env'))) {
  mkdirSync(dirname(appSupportEnv), { recursive: true });
  copyFileSync(join(repoRoot, '.env'), appSupportEnv);
  chmodSync(appSupportEnv, 0o600);
  console.log(`Installed local ${channel.appName} environment to ${appSupportEnv}`);
}

if (releaseTag && existingTag.status !== 0) {
  run('git', ['tag', '-a', releaseTag, installRevision, '-m', `Release ${releaseTag}`]);
}
if (channelName === 'stable' && !releaseTag) {
  console.warn('Override install has no version tag.');
}

console.log(`Installed ${channel.appName} to "${channel.targetApp}"`);
console.log(`Open with: open ${shellQuote(channel.targetApp)}`);

if (shouldOpen) {
  openInstalledApp();
}
