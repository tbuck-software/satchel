import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const script = fileURLToPath(new URL('./build-notarized-macos.sh', import.meta.url));

/** @param {number} validFrom @param {number} [buildExit] */
function runBuild(validFrom, buildExit = 0) {
  const root = mkdtempSync(join(tmpdir(), 'satchel-notarization-'));
  try {
    const tools = join(root, 'tools');
    mkdirSync(tools);
    writeFileSync(join(tools, 'build-app'), `#!/bin/sh
printf 'build\\n' >> "$FIXTURE/calls"
mkdir -p "$FIXTURE/bundles/Satchel Daily.app"
exit "$BUILD_EXIT"
`, { mode: 0o700 });
    writeFileSync(join(tools, 'xcrun'), `#!/bin/sh
test "$1" = stapler && test "$2" = validate || exit 9
printf 'validate:%s\\n' "$3" >> "$FIXTURE/calls"
count=$(cat "$FIXTURE/count" 2>/dev/null || printf 0)
count=$((count + 1))
printf '%s' "$count" > "$FIXTURE/count"
test "$count" -ge "$VALID_FROM"
`, { mode: 0o700 });
    writeFileSync(join(tools, 'sleep'), `#!/bin/sh
printf 'sleep:%s\\n' "$1" >> "$FIXTURE/calls"
`, { mode: 0o700 });
    const result = spawnSync('bash', [script, join(root, 'bundles'), 'build-app'], {
      encoding: 'utf8',
      env: { ...process.env, PATH: `${tools}:${process.env.PATH}`, FIXTURE: root,
        VALID_FROM: String(validFrom), BUILD_EXIT: String(buildExit) },
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

  it('rebuilds every bundle when stapling was silently unsuccessful', () => {
    const result = runBuild(2);
    expect(result.status).toBe(0);
    expect(result.calls).toEqual(['build', `validate:${result.app}`, 'sleep:10', 'build', `validate:${result.app}`]);
  });

  it('stops after three builds when no valid ticket is available', () => {
    const result = runBuild(99);
    expect(result.status).not.toBe(0);
    expect(result.calls.filter(call => call === 'build')).toHaveLength(3);
    expect(result.calls.filter(call => call.startsWith('sleep:'))).toEqual(['sleep:10', 'sleep:20']);
  });

  it('does not retry a failed compile or notarization command', () => {
    const result = runBuild(1, 42);
    expect(result.status).toBe(42);
    expect(result.calls).toEqual(['build']);
  });
});
