#!/usr/bin/env node
import { existsSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

function latestInstalledNdk(androidSdk) {
  const ndkDirectory = join(androidSdk, 'ndk');
  if (!existsSync(ndkDirectory)) return undefined;

  return readdirSync(ndkDirectory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort((left, right) => right.localeCompare(left, undefined, { numeric: true }))[0];
}

function resolveNdkRoot() {
  const configuredNdk = process.env.ANDROID_NDK_HOME ?? process.env.ANDROID_NDK_ROOT;
  if (configuredNdk) return configuredNdk;

  const androidSdk = process.env.ANDROID_HOME
    ?? process.env.ANDROID_SDK_ROOT
    ?? join(homedir(), 'Library', 'Android', 'sdk');
  const ndkVersion = latestInstalledNdk(androidSdk);
  return ndkVersion ? join(androidSdk, 'ndk', ndkVersion) : undefined;
}

function ndkHostTag() {
  if (process.platform === 'darwin') return 'darwin-x86_64';
  if (process.platform === 'linux') return 'linux-x86_64';
  if (process.platform === 'win32') return 'windows-x86_64';
  return undefined;
}

const ndkRoot = resolveNdkRoot();
const hostTag = ndkHostTag();
const executableSuffix = process.platform === 'win32' ? '.exe' : '';
const ranlib = ndkRoot && hostTag
  ? join(ndkRoot, 'toolchains', 'llvm', 'prebuilt', hostTag, 'bin', `llvm-ranlib${executableSuffix}`)
  : undefined;

if (!ranlib || !existsSync(ranlib)) {
  console.error('Could not find the Android NDK llvm-ranlib required to build OpenSSL.');
  console.error('Set ANDROID_NDK_HOME to an installed Android NDK and retry.');
  process.exit(1);
}

const env = { ...process.env };
for (const target of [
  'aarch64_linux_android',
  'armv7_linux_androideabi',
  'i686_linux_android',
  'x86_64_linux_android',
]) {
  env[`RANLIB_${target}`] ??= ranlib;
}

const result = spawnSync('tauri', ['android', 'dev', ...process.argv.slice(2)], {
  stdio: 'inherit',
  env,
});
process.exit(result.status ?? 1);
