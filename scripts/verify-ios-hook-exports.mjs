#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import { existsSync, realpathSync } from 'node:fs';
import { join } from 'node:path';
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
 * One line per hook that `exports` (the output of `dyld_info -exports`) lacks.
 * @param {string} image
 * @param {string} exports
 */
export function hookExportProblems(image, exports) {
  return NATIVE_HOOKS
    .filter((name) => !new RegExp(`(^|\\s)_${name}(\\s|$)`, 'm').test(exports))
    .map((name) => `${image} does not export _${name}`);
}

/** @param {string[]} xcodebuildArgs the arguments the app was built with */
export function verifyIosHookExports(xcodebuildArgs) {
  /** @param {...string} args */
  const xcrun = (...args) => execFileSync('xcrun', args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  const settings = appBuildSettings(xcrun('xcodebuild', '-showBuildSettings', '-json', ...xcodebuildArgs));
  const image = codeImage(settings, existsSync);
  if (!existsSync(image)) throw new Error(`no built image at ${image}`);
  console.log(`Checking native notification hook exports in ${image}`);
  const problems = hookExportProblems(image, xcrun('dyld_info', '-exports', image));
  if (problems.length) throw new Error(problems.join('\n'));
  console.log(`Verified ${NATIVE_HOOKS.length} native notification hook exports in ${image}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  try { verifyIosHookExports(process.argv.slice(2)); }
  catch (error) { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; }
}
