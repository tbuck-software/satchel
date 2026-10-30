#!/usr/bin/env node
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import { existsSync, realpathSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';

/**
 * The C entry points Rust resolves with `dlsym(RTLD_DEFAULT, …)` at runtime
 * (`src-tauri/src/commands/mobile_notifications.rs`). The hook table in the
 * generated `main.mm` is what pulls them out of libapp.a.
 */
export const NATIVE_HOOKS = [
  'init_plugin_satchel_mail_notifications',
  'satchel_mobile_frontend_ready',
  'satchel_mobile_response_handled',
];

/**
 * The application target's settings from `xcodebuild -showBuildSettings -json`.
 * @param {string} output
 * @returns {Record<string, string>}
 */
export function appBuildSettings(output) {
  /** @type {{ buildSettings?: Record<string, string> }[]} */
  const targets = JSON.parse(output);
  const apps = targets
    .map((target) => target.buildSettings ?? {})
    .filter((settings) => settings.PRODUCT_TYPE === 'com.apple.product-type.application');
  if (apps.length !== 1) throw new Error(`expected exactly one application target, found ${apps.length}`);
  return apps[0];
}

/**
 * The image that holds the app's code. Xcode 16 Debug builds link it into
 * `<executable>.debug.dylib` (ENABLE_DEBUG_DYLIB) and leave the executable a
 * stub that loads it; `dlsym(RTLD_DEFAULT, …)` searches that dylib too.
 * @param {Record<string, string>} settings
 * @param {(path: string) => boolean} exists
 */
export function codeImage(settings, exists) {
  const products = settings.BUILT_PRODUCTS_DIR;
  const executable = settings.EXECUTABLE_PATH;
  if (!products || !executable) throw new Error('xcodebuild reported no BUILT_PRODUCTS_DIR or EXECUTABLE_PATH');
  const debugDylib = join(products, settings.EXECUTABLE_DEBUG_DYLIB_PATH || `${executable}.debug.dylib`);
  if (settings.ENABLE_DEBUG_DYLIB === 'YES' && exists(debugDylib)) return debugDylib;
  return join(products, executable);
}

/**
 * The hooks among the lines `dyld_info -exports` prints, one symbol per line
 * after its offset. The debug dylib exports every symbol of libapp.a, more
 * than any output buffer holds, so the lines are read as they arrive and only
 * the hooks are kept.
 * @param {Iterable<string> | AsyncIterable<string>} lines
 * @returns {Promise<Set<string>>}
 */
export async function exportedHooks(lines) {
  const wanted = new Map(NATIVE_HOOKS.map((name) => [`_${name}`, name]));
  const found = new Set();
  for await (const line of lines) {
    for (const token of line.trim().split(/\s+/)) {
      const hook = wanted.get(token);
      if (hook) found.add(hook);
    }
  }
  return found;
}

/**
 * One line per hook missing from `exported`.
 * @param {string} image
 * @param {Set<string>} exported
 */
export function hookExportProblems(image, exported) {
  return NATIVE_HOOKS
    .filter((name) => !exported.has(name))
    .map((name) => `${image} does not export _${name}`);
}

/** @param {string} image */
async function imageExports(image) {
  const listing = spawn('xcrun', ['dyld_info', '-exports', image], { stdio: ['ignore', 'pipe', 'inherit'] });
  // Rejects on a spawn error too; handled once the listing has been read.
  const exited = once(listing, 'close');
  exited.catch(() => {});
  const exported = await exportedHooks(createInterface({ input: listing.stdout, crlfDelay: Infinity }));
  const [code, signal] = await exited;
  if (code !== 0) throw new Error(`xcrun dyld_info -exports failed (${signal ?? `exit ${code}`})`);
  return exported;
}

/** @param {string[]} xcodebuildArgs the arguments the app was built with */
export async function verifyIosHookExports(xcodebuildArgs) {
  const settings = appBuildSettings(execFileSync(
    'xcrun',
    ['xcodebuild', '-showBuildSettings', '-json', ...xcodebuildArgs],
    { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  ));
  const image = codeImage(settings, existsSync);
  if (!existsSync(image)) throw new Error(`no built image at ${image}`);
  console.log(`Checking native notification hook exports in ${image}`);
  const problems = hookExportProblems(image, await imageExports(image));
  if (problems.length) throw new Error(problems.join('\n'));
  console.log(`Verified ${NATIVE_HOOKS.length} native notification hook exports in ${image}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  verifyIosHookExports(process.argv.slice(2)).catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
