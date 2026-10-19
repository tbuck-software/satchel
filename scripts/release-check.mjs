#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const required = [
  'APPLE_ID',
  'APPLE_PASSWORD',
  'APPLE_TEAM_ID',
  'APPLE_SIGNING_IDENTITY',
];

/**
 * @param {string} message
 */
function fail(message) {
  console.error(message);
  process.exit(1);
}

/**
 * @param {string} command
 * @param {string[]} args
 * @param {Omit<import('node:child_process').SpawnSyncOptionsWithStringEncoding, 'encoding'>} [options]
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

/** @param {string} path */
function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

const packageJson = readJson('package.json');
const tauriConfig = readJson('src-tauri/tauri.conf.json');
const cargoToml = readFileSync('src-tauri/Cargo.toml', 'utf8');
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

if (packageJson.version !== tauriConfig.version || packageJson.version !== cargoVersion) {
  fail(
    [
      'Version mismatch:',
      `- package.json: ${packageJson.version}`,
      `- src-tauri/tauri.conf.json: ${tauriConfig.version}`,
      `- src-tauri/Cargo.toml: ${cargoVersion ?? 'missing'}`,
    ].join('\n'),
  );
}

if (tauriConfig.identifier !== 'de.tbuck.satchel') {
  fail(`Stable release identifier must be de.tbuck.satchel, got ${tauriConfig.identifier}`);
}

const missing = required.filter((key) => !process.env[key]?.trim());
if (missing.length > 0) {
  console.error('Missing macOS signing/notarization environment variables:');
  for (const key of missing) console.error(`- ${key}`);
  console.error('\nSee docs/RELEASE.md for one-time Apple Developer setup.');
  process.exit(1);
}

if (process.env.SATCHEL_ALLOW_DIRTY_RELEASE !== '1') {
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8' });
  if (status.status !== 0) {
    fail(status.stderr || 'Failed to inspect git status.');
  }
  if (status.stdout.trim()) {
    fail(
      'Release requires a clean git working tree. Commit or stash changes, or set SATCHEL_ALLOW_DIRTY_RELEASE=1 for a deliberate local test.',
    );
  }
}

run('pnpm', ['run', 'i18n:check']);
run('pnpm', ['exec', 'vue-tsc', '-b']);
run('pnpm', ['exec', 'vitest', 'run', '--maxWorkers=3']);
run('cargo', ['fmt', '--all', '--check']);
run('cargo', ['clippy', '-p', 'satchel', '-p', 'satchel-core', '--all-targets', '--', '-D', 'warnings']);
run('pnpm', ['run', 'plugins:build']);
const gpg = process.env.SATCHEL_GPG_BIN || 'gpg';
run(gpg, ['--version']);
run('cargo', ['test', '-p', 'satchel', '-p', 'satchel-core'], {
  env: { ...process.env, SATCHEL_REQUIRE_PLUGIN_ARTIFACTS: '1', SATCHEL_REQUIRE_GPG: '1', SATCHEL_GPG_BIN: gpg },
});
run('pnpm', ['run', 'build']);
run('pnpm', ['run', 'bundle:check']);
run('pnpm', ['run', 'test:smoke']);
run('pnpm', ['run', 'test:csp']);

console.log('macOS release environment and checks look ready.');
