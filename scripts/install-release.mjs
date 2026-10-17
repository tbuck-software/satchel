#!/usr/bin/env node
// @ts-check
import { execFileSync, spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import config from '../src-tauri/tauri.conf.json' with { type: 'json' };
import { copyProfileMasterKey, removeBackupMasterKey } from './profile-master-key.mjs';
import { processListContainsApp } from './install-mac-helpers.mjs';
import { compareReleaseVersions, PLATFORMS, RELEASE_REPOSITORY, verifyUpdate } from './release-model.mjs';

const args = process.argv.slice(2);
const index = args.indexOf('--channel');
const channel = index >= 0 ? args[index + 1] : 'stable';
if (channel !== 'stable' && channel !== 'nightly') throw new Error('Expected --channel stable or nightly');
if (args.includes('--help')) {
  console.log('Usage: node scripts/install-release.mjs --channel stable|nightly [--open]\nDownloads and verifies the latest macOS release. Private releases use your GitHub CLI login.');
  process.exit(0);
}
if (process.platform !== 'darwin') throw new Error('This installer supports macOS. Linux and Windows installers are on the releases page.');
const target = process.arch === 'arm64' ? 'aarch64-apple-darwin' : process.arch === 'x64' ? 'x86_64-apple-darwin' : null;
if (!target) throw new Error('Unsupported Mac architecture');
const platform = PLATFORMS[target];
const name = channel === 'nightly' ? 'Satchel Daily' : 'Satchel';
const identifier = channel === 'nightly' ? 'de.tbuck.satchel.daily' : 'de.tbuck.satchel';
const installed = join('/Applications', `${name}.app`);
const profile = join(homedir(), 'Library', 'Application Support', identifier, 'satchel');
const temporary = mkdtempSync(join(tmpdir(), 'satchel-release-'));

/** @param {string} tag @param {string} filename @returns {Promise<Buffer>} */
async function download(tag, filename) {
  const url = `https://github.com/${RELEASE_REPOSITORY}/releases/${tag === 'latest' ? 'latest/download' : `download/${tag}`}/${filename}`;
  const response = await fetch(url, { signal: AbortSignal.timeout(120_000) });
  if (response.ok) return Buffer.from(await response.arrayBuffer());
  if (response.status !== 404 && response.status !== 403) throw new Error(`Release download failed (HTTP ${response.status})`);
  const result = spawnSync('gh', ['release', 'download', tag === 'latest' ? '' : tag, '--repo', RELEASE_REPOSITORY, '--pattern', filename, '--dir', temporary, '--clobber'].filter(Boolean), { stdio: ['ignore', 'ignore', 'pipe'] });
  if (result.status !== 0) throw new Error('Private releases need an authenticated GitHub CLI. Run gh auth login, then retry.');
  return readFileSync(join(temporary, filename));
}

function running() {
  const ps = spawnSync('ps', ['ax', '-o', 'command='], { encoding: 'utf8' });
  return ps.status === 0 && processListContainsApp(ps.stdout, installed, 'satchel');
}

async function quit() {
  if (!running()) return;
  spawnSync('osascript', ['-e', `tell application id "${identifier}" to quit`], { stdio: 'ignore' });
  for (let count = 0; count < 50 && running(); count += 1) await new Promise(resolve => setTimeout(resolve, 200));
  if (running()) throw new Error(`Quit ${name}, save any open drafts, and retry.`);
}

function backup() {
  const db = join(profile, 'satchel.db');
  if (!existsSync(db)) return;
  const backups = join(profile, 'backups');
  const destination = join(backups, 'pre-install-' + new Date().toISOString().replace(/[:.]/g, '-'));
  mkdirSync(destination, { recursive: true, mode: 0o700 });
  const snapshot = join(destination, 'satchel.db');
  execFileSync('sqlite3', [db, `.backup '${snapshot.replace(/'/g, "''")}'`], { stdio: 'inherit' });
  copyProfileMasterKey(profile, destination, { backup: true });
  const previous = readdirSync(backups).filter(value => /^pre-(install|import)-/.test(value))
    .sort((a, b) => b.replace(/^pre-(install|import)-/, '').localeCompare(a.replace(/^pre-(install|import)-/, '')));
  for (const old of previous.slice(3)) {
    removeBackupMasterKey(join(backups, old));
    rmSync(join(backups, old), { recursive: true, force: true });
  }
  console.log('Created a consistent database and keychain backup.');
}

try {
  console.log(`Downloading the latest ${channel} build…`);
  const manifest = JSON.parse((await download(channel === 'nightly' ? 'nightly' : 'latest', 'latest.json')).toString('utf8'));
  if (!/^\d+\.\d+\.\d+(?:-nightly\.\d+)?$/.test(manifest.version)
    || manifest.release_tag !== `v${manifest.version}` || manifest.channel !== channel) throw new Error('Invalid release manifest');
  const asset = manifest.platforms[platform.platform];
  const filename = `Satchel${channel === 'nightly' ? '-Daily' : ''}-${platform.name}${platform.updater}`;
  const expectedUrl = `https://github.com/${RELEASE_REPOSITORY}/releases/download/${manifest.release_tag}/${filename}`;
  if (!asset || asset.url !== expectedUrl || typeof asset.signature !== 'string') throw new Error('Invalid update asset');
  const bytes = await download(manifest.release_tag, filename);
  verifyUpdate(bytes, asset.signature, config.plugins.updater.pubkey);
  const archive = join(temporary, filename);
  writeFileSync(archive, bytes, { mode: 0o600 });
  execFileSync('tar', ['-xzf', archive, '-C', temporary]);
  const app = join(temporary, `${name}.app`);
  const plist = join(app, 'Contents', 'Info.plist');
  const bundleId = execFileSync('plutil', ['-extract', 'CFBundleIdentifier', 'raw', '-o', '-', plist], { encoding: 'utf8' }).trim();
  if (bundleId !== identifier || !existsSync(join(app, 'Contents', 'MacOS', 'satchel'))) throw new Error('Release belongs to another app or channel');
  const shipped = JSON.parse(readFileSync(join(app, 'Contents', 'Resources', 'release.json'), 'utf8'));
  if (shipped.version !== manifest.version || shipped.channel !== channel || shipped.tag !== manifest.release_tag || shipped.source !== manifest.source) {
    throw new Error('The signed app does not match the announced release');
  }
  execFileSync('codesign', ['--verify', '--deep', '--strict', app], { stdio: 'inherit' });
  if (existsSync(installed)) {
    const receipt = join(installed, 'Contents', 'Resources', 'release.json');
    const previous = existsSync(receipt)
      ? JSON.parse(readFileSync(receipt, 'utf8')).version
      : execFileSync('plutil', ['-extract', 'CFBundleShortVersionString', 'raw', '-o', '-', join(installed, 'Contents', 'Info.plist')], { encoding: 'utf8' }).trim();
    if (compareReleaseVersions(manifest.version, previous) < 0) throw new Error('The installed app is newer. Refusing to downgrade its database.');
  }
  const staged = `${installed}.installing-${process.pid}`;
  const old = `${installed}.previous-${process.pid}`;
  try {
    cpSync(app, staged, { recursive: true, errorOnExist: true, force: false });
    // The Tauri signature was checked before extraction; retain the app's own code signature.
    execFileSync('xattr', ['-dr', 'com.apple.quarantine', staged]);
    await quit();
    backup();
    if (existsSync(installed)) renameSync(installed, old);
    renameSync(staged, installed);
    rmSync(old, { recursive: true, force: true });
  } catch (error) {
    if (!existsSync(installed) && existsSync(old)) renameSync(old, installed);
    rmSync(staged, { recursive: true, force: true });
    throw error;
  }
  if (args.includes('--open')) execFileSync('open', [installed]);
  console.log(`Installed ${name} ${manifest.version} in ${installed}.`);
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
