import { describe, expect, it } from 'vitest';
import { appBuildSettings, codeImage, hookExportProblems, NATIVE_HOOKS } from './verify-ios-hook-exports.mjs';

const settings = {
  BUILT_PRODUCTS_DIR: '/DerivedData/satchel-abc/Build/Products/debug-iphonesimulator',
  EXECUTABLE_PATH: 'Satchel.app/Satchel',
  ENABLE_DEBUG_DYLIB: 'YES',
  EXECUTABLE_DEBUG_DYLIB_PATH: 'Satchel.app/Satchel.debug.dylib',
};
const products = settings.BUILT_PRODUCTS_DIR;
/** `dyld_info -exports` lists one symbol per line after its offset. */
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

  it('reports every missing hook and the image it checked', () => {
    const problems = hookExportProblems(`${products}/Satchel.app/Satchel`, exportsOf(['satchel_mobile_frontend_ready']));
    expect(problems).toEqual([
      `${products}/Satchel.app/Satchel does not export _init_plugin_satchel_mail_notifications`,
      `${products}/Satchel.app/Satchel does not export _satchel_mobile_response_handled`,
    ]);
  });

  it('accepts an image that exports all hooks, and no near-miss names', () => {
    expect(hookExportProblems('image', exportsOf(NATIVE_HOOKS))).toEqual([]);
    const nearMisses = NATIVE_HOOKS.map((name) => `${name}_v2`);
    expect(hookExportProblems('image', exportsOf(nearMisses))).toHaveLength(NATIVE_HOOKS.length);
  });

  it('reads the application target from xcodebuild -showBuildSettings -json', () => {
    const output = JSON.stringify([
      { target: 'Tests', buildSettings: { PRODUCT_TYPE: 'com.apple.product-type.bundle.unit-test' } },
      { target: 'satchel_iOS', buildSettings: { ...settings, PRODUCT_TYPE: 'com.apple.product-type.application' } },
    ]);
    expect(appBuildSettings(output)).toMatchObject(settings);
    expect(() => appBuildSettings('[]')).toThrow('exactly one application target');
  });
});
