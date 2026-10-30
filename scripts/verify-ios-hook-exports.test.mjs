import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import { describe, expect, it, onTestFinished, vi } from 'vitest';
import {
  appBuildSettings,
  codeImage,
  exportedHooks,
  hookExportProblems,
  NATIVE_HOOKS,
  verifyIosHookExports,
} from './verify-ios-hook-exports.mjs';

const settings = {
  BUILT_PRODUCTS_DIR: '/DerivedData/satchel-abc/Build/Products/debug-iphonesimulator',
  EXECUTABLE_PATH: 'Satchel.app/Satchel',
  ENABLE_DEBUG_DYLIB: 'YES',
  EXECUTABLE_DEBUG_DYLIB_PATH: 'Satchel.app/Satchel.debug.dylib',
};
const products = settings.BUILT_PRODUCTS_DIR;
/** `dyld_info -exports` lists one symbol per line after its offset. */
const lines = (/** @type {string} */ text) => text.split('\n');
const exportsOf = (/** @type {string[]} */ names) =>
  `${products}/Satchel.app/Satchel.debug.dylib [arm64]:\n    -exports:\n${names.map((name, index) => `        0x0000${index}000  _${name}`).join('\n')}\n`;

describe('iOS notification hook export check', () => {
  it('checks the debug dylib that holds the code when Xcode built one', () => {
    expect(codeImage(settings, () => true)).toBe(`${products}/Satchel.app/Satchel.debug.dylib`);
  });

  it('checks the executable when no debug dylib was built', () => {
    expect(codeImage(settings, () => false)).toBe(`${products}/Satchel.app/Satchel`);
    expect(codeImage({ ...settings, ENABLE_DEBUG_DYLIB: 'NO' }, () => true)).toBe(`${products}/Satchel.app/Satchel`);
  });

  it('reports every missing hook and the image it checked', async () => {
    const exported = await exportedHooks(lines(exportsOf(['satchel_mobile_frontend_ready'])));
    expect(hookExportProblems(`${products}/Satchel.app/Satchel`, exported)).toEqual([
      `${products}/Satchel.app/Satchel does not export _init_plugin_satchel_mail_notifications`,
      `${products}/Satchel.app/Satchel does not export _satchel_mobile_response_handled`,
    ]);
  });

  it('accepts an image that exports all hooks, and no near-miss names', async () => {
    expect(hookExportProblems('image', await exportedHooks(lines(exportsOf(NATIVE_HOOKS))))).toEqual([]);
    const nearMisses = NATIVE_HOOKS.map((name) => `${name}_v2`);
    expect(hookExportProblems('image', await exportedHooks(lines(exportsOf(nearMisses))))).toHaveLength(NATIVE_HOOKS.length);
  });

  // The 7f504c4 simulator build died with `spawnSync xcrun ENOBUFS`: a debug
  // dylib exports every symbol of libapp.a, more than the 64 MiB buffer held.
  it('reads an export listing larger than any output buffer', async () => {
    const root = mkdtempSync(join(tmpdir(), 'satchel-hook-exports-'));
    onTestFinished(() => rmSync(root, { recursive: true, force: true }));
    const built = join(root, 'Products');
    mkdirSync(join(built, 'Satchel.app'), { recursive: true });
    writeFileSync(join(built, 'Satchel.app', 'Satchel.debug.dylib'), '');
    const buildSettings = [{ buildSettings: {
      ...settings, BUILT_PRODUCTS_DIR: built, PRODUCT_TYPE: 'com.apple.product-type.application',
    } }];
    const xcrun = join(root, 'xcrun');
    writeFileSync(xcrun, `#!/usr/bin/env node
const out = (chunk) => new Promise((resolve) => (process.stdout.write(chunk) ? resolve() : process.stdout.once('drain', resolve)));
(async () => {
  if (process.argv[2] === 'xcodebuild') return out(${JSON.stringify(JSON.stringify(buildSettings))});
  const filler = ('        0x00001000  _ZN' + 'x'.repeat(230) + 'E\\n').repeat(4096);
  for (let mib = 0; mib < 72; mib += 1) await out(filler);
  await out(${JSON.stringify(exportsOf(NATIVE_HOOKS))});
})();
`);
    chmodSync(xcrun, 0o755);
    vi.stubEnv('PATH', `${root}${delimiter}${process.env.PATH ?? ''}`);
    onTestFinished(() => { vi.unstubAllEnvs(); });
    const log = vi.spyOn(console, 'log').mockImplementation(() => {});
    onTestFinished(() => { log.mockRestore(); });

    await expect(verifyIosHookExports(['-scheme', 'satchel_iOS'])).resolves.toBeUndefined();
    expect(log).toHaveBeenLastCalledWith(expect.stringContaining(`Verified ${NATIVE_HOOKS.length} native notification hook exports`));
  }, 30_000);

  it('reads the application target from xcodebuild -showBuildSettings -json', () => {
    const output = JSON.stringify([
      { target: 'Tests', buildSettings: { PRODUCT_TYPE: 'com.apple.product-type.bundle.unit-test' } },
      { target: 'satchel_iOS', buildSettings: { ...settings, PRODUCT_TYPE: 'com.apple.product-type.application' } },
    ]);
    expect(appBuildSettings(output)).toMatchObject(settings);
    expect(() => appBuildSettings('[]')).toThrow('exactly one application target');
  });
});
