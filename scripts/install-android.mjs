#!/usr/bin/env node
import { existsSync, readdirSync, rmSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { provisionAndroidOAuthConfig } from './android-oauth-config.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const packageId = 'de.tbuck.satchel';
const shouldOpen = process.argv.includes('--open');

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
 * @param {import('node:child_process').SpawnSyncOptions} [options]
 */
function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    ...options,
  });
  if (result.status !== 0) process.exit(result.status ?? 1);
  return result;
}

/**
 * @param {string} command
 * @param {string[]} args
 * @param {Omit<import('node:child_process').SpawnSyncOptionsWithStringEncoding, 'encoding'>} [options]
 */
function capture(command, args, options = {}) {
  return spawnSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    ...options,
  });
}

/**
 * @param {string} androidSdk
 */
function latestInstalledNdk(androidSdk) {
  const directory = join(androidSdk, 'ndk');
  if (!existsSync(directory)) return undefined;
  return readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort((left, right) => right.localeCompare(left, undefined, { numeric: true }))[0];
}

function androidSdkRoot() {
  return process.env.ANDROID_HOME
    ?? process.env.ANDROID_SDK_ROOT
    ?? join(homedir(), 'Library', 'Android', 'sdk');
}

function resolveAdb() {
  const sdkAdb = join(androidSdkRoot(), 'platform-tools', process.platform === 'win32' ? 'adb.exe' : 'adb');
  return existsSync(sdkAdb) ? sdkAdb : 'adb';
}

function androidBuildEnv() {
  const sdk = androidSdkRoot();
  const ndkRoot = process.env.ANDROID_NDK_HOME
    ?? process.env.ANDROID_NDK_ROOT
    ?? (latestInstalledNdk(sdk) ? join(sdk, 'ndk', latestInstalledNdk(sdk)) : undefined);
  const hostTag = process.platform === 'darwin'
    ? 'darwin-x86_64'
    : process.platform === 'linux'
      ? 'linux-x86_64'
      : process.platform === 'win32'
        ? 'windows-x86_64'
        : undefined;
  const ranlib = ndkRoot && hostTag
    ? join(ndkRoot, 'toolchains', 'llvm', 'prebuilt', hostTag, 'bin', `llvm-ranlib${process.platform === 'win32' ? '.exe' : ''}`)
    : undefined;
  if (!ranlib || !existsSync(ranlib)) {
    fail('Could not find the Android NDK llvm-ranlib. Set ANDROID_NDK_HOME and retry.');
  }

  /** @type {NodeJS.ProcessEnv} */
  const env = { ...process.env, ANDROID_HOME: sdk, ANDROID_NDK_HOME: ndkRoot };
  for (const target of [
    'aarch64_linux_android',
    'armv7_linux_androideabi',
    'i686_linux_android',
    'x86_64_linux_android',
  ]) {
    env[`RANLIB_${target}`] ??= ranlib;
  }
  return env;
}

/**
 * @param {string} adb
 */
function connectedDevices(adb) {
  const result = capture(adb, ['devices', '-l']);
  if (result.status !== 0) fail(result.stderr.trim() || 'Could not query Android devices.');
  return result.stdout.split(/\r?\n/).slice(1).flatMap((line) => {
    const match = line.trim().match(/^(\S+)\s+device(?:\s|$)/);
    return match ? [match[1]] : [];
  });
}

/**
 * @param {string} adb
 */
function connectDiscoveredDevice(adb) {
  const result = capture(adb, ['mdns', 'services']);
  if (result.status !== 0) return;
  const endpoints = result.stdout.split(/\r?\n/).flatMap((line) => {
    const match = line.match(/_adb-tls-connect\._tcp\s+(\S+:\d+)\s*$/);
    return match ? [match[1]] : [];
  });
  for (const endpoint of endpoints) {
    const connection = capture(adb, ['connect', endpoint]);
    if (connection.status === 0 && /connected to|already connected/.test(connection.stdout)) return;
  }
}

/**
 * @param {string} adb
 */
function selectDevice(adb) {
  let devices = connectedDevices(adb);
  if (devices.length === 0) {
    connectDiscoveredDevice(adb);
    devices = connectedDevices(adb);
  }
  const requested = process.env.ANDROID_SERIAL;
  if (requested) {
    if (!devices.includes(requested)) fail(`ANDROID_SERIAL=${requested} is not connected.`);
    return requested;
  }
  if (devices.length === 0) {
    fail('No Android device is connected. Connect USB or enable Wireless debugging and retry.');
  }
  if (devices.length > 1) {
    fail(`Multiple Android devices are connected (${devices.join(', ')}). Set ANDROID_SERIAL and retry.`);
  }
  return devices[0];
}

/**
 * @param {string} abi
 */
function tauriTargetForAbi(abi) {
  /** @type {Record<string, string | undefined>} */
  const targets = {
    'arm64-v8a': 'aarch64',
    'armeabi-v7a': 'armv7',
    x86: 'i686',
    x86_64: 'x86_64',
  };
  return targets[abi];
}

/**
 * @param {string} directory
 * @returns {string[]}
 */
function findApks(directory) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return findApks(path);
    return entry.isFile() && entry.name.endsWith('-debug.apk') ? [path] : [];
  });
}

/**
 * @param {string} adb
 * @param {string} serial
 * @param {string} packageName
 */
function isPackageForeground(adb, serial, packageName) {
  const result = capture(adb, ['-s', serial, 'shell', 'dumpsys', 'activity', 'activities']);
  if (result.status !== 0) return false;
  const escapedPackage = packageName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(
    `(?:mResumedActivity|topResumedActivity|ResumedActivity)[^\\n]*${escapedPackage}`,
  ).test(result.stdout);
}

/**
 * @param {string} adb
 * @param {string} serial
 * @param {string} packageName
 */
function waitForPackageRestore(adb, serial, packageName, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  const waiter = new Int32Array(new SharedArrayBuffer(4));
  while (Date.now() < deadline) {
    if (isPackageForeground(adb, serial, packageName)) return true;
    Atomics.wait(waiter, 0, 0, 250);
  }
  return false;
}

const adb = resolveAdb();
const serial = selectDevice(adb);
const abiResult = capture(adb, ['-s', serial, 'shell', 'getprop', 'ro.product.cpu.abi']);
if (abiResult.status !== 0) fail(`Could not determine the ABI of ${serial}.`);
const abi = abiResult.stdout.trim();
const target = tauriTargetForAbi(abi);
if (!target) fail(`Unsupported Android ABI reported by ${serial}: ${abi || 'unknown'}`);

const modelResult = capture(adb, ['-s', serial, 'shell', 'getprop', 'ro.product.model']);
const model = modelResult.status === 0 ? modelResult.stdout.trim() : serial;
const apkRoot = join(repoRoot, 'src-tauri', 'gen', 'android', 'app', 'build', 'outputs', 'apk');

// Tauri can switch between split and universal output without cleaning the
// other directory. Remove only generated APK outputs so a successful build
// can never be followed by installation of a stale artifact.
rmSync(apkRoot, { recursive: true, force: true });

console.log(`Building Satchel for ${model} (${abi})...`);
run(
  'npx',
  ['--yes', '@tauri-apps/cli@^2', 'android', 'build', '--debug', '--apk', '--target', target, '--ci'],
  { env: androidBuildEnv() },
);

const apks = findApks(apkRoot);
const preferredName = `app-${abi.replace('-v8a', '').replace('armeabi-v7a', 'arm32')}-debug.apk`;
const apk = apks.find((candidate) => basename(candidate) === preferredName)
  ?? apks.find((candidate) => candidate.includes(`/${abi === 'arm64-v8a' ? 'arm64' : target}/`))
  ?? apks.find((candidate) => basename(candidate) === 'app-universal-debug.apk');
if (!apk) fail(`Android build finished, but no fresh debug APK was found below ${apkRoot}.`);

// Android 17 restores an Activity that was visible across a package update.
// Launching it ourselves while that transition is pending can create Wry twice
// in one process and make ndk-context abort on its second initialization.
const wasForeground = shouldOpen && isPackageForeground(adb, serial, packageId);
console.log(`Installing ${basename(apk)} on ${model}...`);
run(adb, ['-s', serial, 'install', '-r', apk]);
console.log(`Installed Satchel on ${model}.`);

const oauthConfigured = provisionAndroidOAuthConfig(adb, serial, packageId, repoRoot);
if (oauthConfigured) {
  console.log('Configured OAuth in private Android app storage.');
  // Runtime configuration is read at startup. End any Activity Android has
  // already restored so its old environment cannot survive the update.
  run(adb, ['-s', serial, 'shell', 'am', 'force-stop', packageId]);
}

if (shouldOpen) {
  if (!oauthConfigured && wasForeground && waitForPackageRestore(adb, serial, packageId)) {
    console.log(`Android restored Satchel on ${model}.`);
  } else {
    run(adb, [
      '-s', serial, 'shell', 'monkey', '-p', packageId,
      '-c', 'android.intent.category.LAUNCHER', '1',
    ]);
    console.log(`Opened Satchel on ${model}.`);
  }
}
