import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

const roots: string[] = [];
afterEach(() => roots.splice(0).forEach((root) => rmSync(root, { recursive: true, force: true })));

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'satchel-android-oauth-'));
  roots.push(root);
  const repo = join(root, 'mobile worktree');
  const main = join(root, 'main checkout');
  const sdk = join(root, 'sdk');
  const bin = join(root, 'bin');
  const profile = join(root, 'profile');
  const host = process.platform === 'darwin' ? 'darwin-x86_64' : 'linux-x86_64';
  for (const directory of [join(repo, 'scripts'), join(main, '.git/worktrees/mobile'), join(sdk, 'platform-tools'), join(sdk, `ndk/28/toolchains/llvm/prebuilt/${host}/bin`), bin, profile]) {
    mkdirSync(directory, { recursive: true });
  }
  writeFileSync(join(repo, '.git'), `gitdir: ${join(main, '.git/worktrees/mobile')}\n`);
  for (const name of ['install-android.mjs', 'android-oauth-config.mjs']) {
    const source = new URL(`../../scripts/${name}`, import.meta.url);
    if (existsSync(source)) copyFileSync(source, join(repo, 'scripts', name));
  }
  writeFileSync(join(sdk, `ndk/28/toolchains/llvm/prebuilt/${host}/bin/llvm-ranlib`), '');
  const adb = join(sdk, 'platform-tools/adb');
  writeFileSync(adb, `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.FIXTURE_EVENTS, JSON.stringify(args) + '\\n');
if (args[0] === 'devices') console.log('List of devices attached\\nfixture-pixel device');
else if (args.includes('ro.product.cpu.abi')) console.log('arm64-v8a');
else if (args.includes('ro.product.model')) console.log('Pixel fixture');
else if (args.includes('run-as')) {
  const contents = fs.readFileSync(0, 'utf8');
  fs.mkdirSync(path.join(process.env.FIXTURE_PROFILE, 'satchel'), { recursive: true });
  fs.writeFileSync(path.join(process.env.FIXTURE_PROFILE, 'satchel/.env'), contents);
}
`);
  chmodSync(adb, 0o755);
  const npx = join(bin, 'npx');
  writeFileSync(npx, `#!/usr/bin/env node
const fs = require('node:fs');
const directory = 'src-tauri/gen/android/app/build/outputs/apk/universal/debug';
fs.mkdirSync(directory, { recursive: true });
fs.writeFileSync(directory + '/app-universal-debug.apk', 'fixture');
`);
  chmodSync(npx, 0o755);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('SATCHEL_')));
  return {
    repo, main, profile,
    install(extraEnv: Record<string, string> = {}) {
      const output = execFileSync(process.execPath, [join(repo, 'scripts/install-android.mjs'), '--open'], {
        cwd: repo,
        encoding: 'utf8',
        env: {
          ...env, ...extraEnv,
          PATH: `${bin}:${env.PATH ?? ''}`,
          ANDROID_HOME: sdk,
          ANDROID_NDK_HOME: join(sdk, 'ndk/28'),
          ANDROID_SERIAL: 'fixture-pixel',
          FIXTURE_EVENTS: join(root, 'events.jsonl'),
          FIXTURE_PROFILE: profile,
        },
      });
      const events = readFileSync(join(root, 'events.jsonl'), 'utf8').trim().split('\n').map((line) => JSON.parse(line) as string[]);
      return { output, events };
    },
  };
}

describe('personal Android OAuth installation', () => {
  it('provisions the main checkout configuration privately and restarts before opening', () => {
    const setup = fixture();
    writeFileSync(join(setup.main, '.env'), 'SATCHEL_GMAIL_CLIENT_ID=fixture-client\nSATCHEL_GMAIL_CLIENT_SECRET=fixture-secret\nUNRELATED_SECRET=must-not-copy\n');
    const { output, events } = setup.install();
    const privateEnv = join(setup.profile, 'satchel/.env');
    expect(existsSync(privateEnv), 'Google refresh needs its runtime client secret on the phone').toBe(true);
    const contents = readFileSync(privateEnv, 'utf8');
    expect(contents).toContain('fixture-client');
    expect(contents).toContain('fixture-secret');
    expect(contents).not.toContain('must-not-copy');
    expect(output).not.toContain('fixture-secret');
    expect(JSON.stringify(events)).not.toContain('fixture-secret');
    const provision = events.findIndex((args) => args.includes('run-as'));
    const stop = events.findIndex((args) => args.includes('force-stop'));
    const open = events.findIndex((args) => args.includes('monkey'));
    expect(provision).toBeGreaterThan(events.findIndex((args) => args.includes('install')));
    expect(events[provision]?.join(' ')).toContain('umask 077');
    expect(events[provision]?.join(' ')).toContain('chmod 600');
    expect(stop).toBeGreaterThan(provision);
    expect(open).toBeGreaterThan(stop);
  });

  it('honors a worktree-local configuration and shell overrides', () => {
    const setup = fixture();
    writeFileSync(join(setup.main, '.env'), 'SATCHEL_GMAIL_CLIENT_SECRET=main-secret\n');
    writeFileSync(join(setup.repo, '.env'), 'SATCHEL_GMAIL_CLIENT_ID=local-client\nSATCHEL_GMAIL_CLIENT_SECRET=local-secret\n');
    setup.install({ SATCHEL_GMAIL_CLIENT_SECRET: 'shell-secret' });
    const contents = readFileSync(join(setup.profile, 'satchel/.env'), 'utf8');
    expect(contents).toContain('local-client');
    expect(contents).toContain('shell-secret');
    expect(contents).not.toContain('main-secret');
    expect(contents).not.toContain('local-secret');
  });

  it('can still install without local OAuth configuration', () => {
    const setup = fixture();
    const { events } = setup.install();
    expect(events.some((args) => args.includes('install'))).toBe(true);
    expect(events.some((args) => args.includes('run-as'))).toBe(false);
    expect(events.some((args) => args.includes('force-stop'))).toBe(false);
  });
});
