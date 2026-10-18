import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const script = fileURLToPath(new URL('./build-notarized-macos.sh', import.meta.url));

/** @param {number} validFrom @param {number} [buildExit] @param {number} [bundleExit] @param {string} [output] @param {number} [stapleValidFrom] */
function runBuild(validFrom, buildExit = 0, bundleExit = 0, output = '', stapleValidFrom = 0) {
  const root = mkdtempSync(join(tmpdir(), 'satchel-notarization-'));
  try {
    const tools = join(root, 'tools');
    mkdirSync(tools);
    writeFileSync(join(tools, 'pnpm'), `#!/bin/sh
test "$1" = exec && test "$2" = tauri && test "$4" = --ci && test "$5" = --bundles && test "$6" = app,dmg || exit 9
printf '%s\\n' "$3" >> "$FIXTURE/calls"
mkdir -p "$FIXTURE/bundles/Satchel Daily.app"
printf '%s\\n' "$PACKAGING_OUTPUT"
if [ "$STAPLE_VALID_FROM" -gt 0 ]; then
  xcrun notarytool submit fixture.zip --key fixture-key-file || exit 9
  xcrun stapler staple -v "$FIXTURE/bundles/Satchel Daily.app" >/dev/null 2>&1 || true
fi
if [ "$3" = bundle ]; then exit "$BUNDLE_EXIT"; fi
exit "$BUILD_EXIT"
`, { mode: 0o700 });
    writeFileSync(join(tools, 'xcrun'), `#!/bin/sh
if [ "$1" = notarytool ]; then
  test "$2" = submit && test "$3" = fixture.zip && test "$4" = --key && test "$5" = fixture-key-file || exit 9
  printf 'notarytool\\n' >> "$FIXTURE/calls"
  exit 0
fi
test "$1" = stapler || exit 9
if [ "$2" = staple ]; then
  printf 'staple\\n' >> "$FIXTURE/calls"
  count=$(cat "$FIXTURE/staple-count" 2>/dev/null || printf 0)
  count=$((count + 1))
  printf '%s' "$count" > "$FIXTURE/staple-count"
  test "$count" -ge "$STAPLE_VALID_FROM" || exit 68
  touch "$FIXTURE/ticket"
  exit 0
fi
test "$2" = validate || exit 9
printf 'validate:%s\\n' "$3" >> "$FIXTURE/calls"
if [ "$STAPLE_VALID_FROM" -gt 0 ]; then test -f "$FIXTURE/ticket"; exit $?; fi
count=$(cat "$FIXTURE/count" 2>/dev/null || printf 0)
count=$((count + 1))
printf '%s' "$count" > "$FIXTURE/count"
test "$count" -ge "$VALID_FROM"
`, { mode: 0o700 });
    writeFileSync(join(tools, 'sleep'), `#!/bin/sh
printf 'sleep:%s\\n' "$1" >> "$FIXTURE/calls"
`, { mode: 0o700 });
    const result = spawnSync('bash', [script, join(root, 'bundles'), '--ci', '--bundles', 'app,dmg'], {
      encoding: 'utf8',
      env: { ...process.env, PATH: `${tools}:${process.env.PATH}`, FIXTURE: root,
        VALID_FROM: String(validFrom), BUILD_EXIT: String(buildExit), BUNDLE_EXIT: String(bundleExit), PACKAGING_OUTPUT: output,
        STAPLE_VALID_FROM: String(stapleValidFrom) },
    });
    return { status: result.status, calls: readFileSync(join(root, 'calls'), 'utf8').trim().split('\n'),
      app: join(root, 'bundles/Satchel Daily.app') };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

describe.skipIf(process.platform === 'win32')('macOS packaging with verified notarization tickets', () => {
  it('accepts a stapled app without rebuilding', () => {
    const result = runBuild(1);
    expect(result.status).toBe(0);
    expect(result.calls).toEqual(['build', `validate:${result.app}`]);
  });

  it('recreates every bundle without recompiling when stapling was silently unsuccessful', () => {
    const result = runBuild(2);
    expect(result.status).toBe(0);
    expect(result.calls).toEqual(['build', `validate:${result.app}`, 'sleep:10', 'bundle', `validate:${result.app}`]);
  });

  it('stops after three bundle attempts when no valid ticket is available', () => {
    const result = runBuild(99);
    expect(result.status).not.toBe(0);
    expect(result.calls.filter(call => call === 'build')).toHaveLength(1);
    expect(result.calls.filter(call => call === 'bundle')).toHaveLength(2);
    expect(result.calls.filter(call => call.startsWith('sleep:'))).toEqual(['sleep:10', 'sleep:20']);
  });

  it('does not retry a failed compile or notarization command', () => {
    const result = runBuild(1, 42);
    expect(result.status).toBe(42);
    expect(result.calls).toEqual(['build']);
  });

  it('stops immediately when rebundling itself fails', () => {
    const result = runBuild(99, 0, 42);
    expect(result.status).toBe(42);
    expect(result.calls).toEqual(['build', `validate:${result.app}`, 'sleep:10', 'bundle']);
  });

  for (const failure of ['A timestamp was expected but was not found.', 'hdiutil: create failed - Resource busy']) {
    it(`retries packaging without compiling again after ${failure}`, () => {
      const result = runBuild(1, 42, 0, `failed to bundle project: ${failure}`);
      expect(result.status).toBe(0);
      expect(result.calls).toEqual(['build', 'sleep:10', 'bundle', `validate:${result.app}`]);
    });
  }

  it('stops after three attempts if a transient packaging failure persists', () => {
    const result = runBuild(1, 42, 42, 'failed to bundle project: hdiutil: create failed - Resource busy');
    expect(result.status).toBe(42);
    expect(result.calls).toEqual(['build', 'sleep:10', 'bundle', 'sleep:20', 'bundle']);
  });

  it('retries stapling before Tauri silently proceeds to create the containers', () => {
    const result = runBuild(1, 0, 0, '', 2);
    expect(result.status).toBe(0);
    expect(result.calls).toEqual(['build', 'notarytool', 'staple', 'sleep:10', 'staple', `validate:${result.app}`]);
  });

  it('blocks publication when stapling never succeeds', () => {
    const result = runBuild(1, 0, 0, '', 99);
    expect(result.status).not.toBe(0);
    expect(result.calls.filter(call => call === 'build')).toHaveLength(1);
    expect(result.calls.filter(call => call === 'staple')).toHaveLength(9);
  });
});
