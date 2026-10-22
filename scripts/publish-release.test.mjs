import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { PLATFORMS } from './release-model.mjs';

const collect = fileURLToPath(new URL('./collect-release.mjs', import.meta.url));
const publish = fileURLToPath(new URL('./publish-release.mjs', import.meta.url));

/** @param {'stable' | 'nightly'} channel */
function releaseFixture(channel) {
  const root = mkdtempSync(join(tmpdir(), 'satchel-publish-release-'));
  const version = channel === 'stable' ? '0.1.2' : '0.1.2-nightly.42';
  const plan = { version, tag: `v${version}`, channel, source: 'a'.repeat(40), published: '2026-10-22T03:14:00Z' };
  mkdirSync(join(root, 'output/installer'), { recursive: true });
  mkdirSync(join(root, 'output/release-assets'), { recursive: true });
  mkdirSync(join(root, 'bin'));
  writeFileSync(join(root, 'output/release-plan.json'), JSON.stringify(plan));
  writeFileSync(join(root, 'output/installer/install-satchel.mjs'), '// fixture installer\n');
  writeFileSync(join(root, 'LICENSE'), 'MIT fixture');
  writeFileSync(join(root, 'THIRD_PARTY_NOTICES.md'), 'Fixture notices');
  const calls = join(root, 'calls.jsonl');
  writeFileSync(join(root, 'bin/gh'), `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.FAKE_RELEASE_CALLS, JSON.stringify(args) + '\\n');
if (args[0] === 'api') console.log(JSON.stringify([{ref: ${JSON.stringify(`refs/tags/${plan.tag}`)}, object: {type: 'commit', sha: ${JSON.stringify(plan.source)}}}]));
`, { mode: 0o700 });
  const env = { ...process.env, PATH: `${join(root, 'bin')}:${process.env.PATH}`, FAKE_RELEASE_CALLS: calls,
    GITHUB_REPOSITORY: 'tbuck-software/satchel', GITHUB_REF: 'refs/heads/main' };
  for (const [target, platform] of Object.entries(PLATFORMS)) {
    const bundle = join(root, 'target', target, 'release/bundle');
    mkdirSync(bundle, { recursive: true });
    writeFileSync(join(bundle, `Satchel${platform.updater}`), 'signed package fixture');
    writeFileSync(join(bundle, `Satchel${platform.updater}.sig`), 'signature fixture');
    if (platform.installer) writeFileSync(join(bundle, `Satchel${platform.installer}`), 'installer fixture');
    execFileSync(process.execPath, [collect, target], { cwd: root, env, stdio: 'pipe' });
    const artifacts = join(root, 'output/release-artifacts', target);
    for (const name of readdirSync(artifacts)) copyFileSync(join(artifacts, name), join(root, 'output/release-assets', name));
  }
  return { root, plan, env, calls };
}

describe('publishing update channels', () => {
  for (const channel of /** @type {const} */ (['stable', 'nightly'])) {
    it(`publishes ${channel} with the regular app identity on every desktop platform`, () => {
      const fixture = releaseFixture(channel);
      try {
        execFileSync(process.execPath, [publish], { cwd: fixture.root, env: fixture.env, stdio: 'pipe' });
        const manifest = JSON.parse(readFileSync(join(fixture.root, 'output/release-assets/latest.json'), 'utf8'));
        expect(manifest.channel).toBe(channel);
        expect(manifest.app_identifier).toBe('de.tbuck.satchel');
        for (const platform of Object.values(PLATFORMS)) {
          expect(manifest.platforms[platform.platform].url).toBe(
            `https://github.com/tbuck-software/satchel/releases/download/${fixture.plan.tag}/Satchel-${platform.name}${platform.updater}`,
          );
        }
        const calls = readFileSync(fixture.calls, 'utf8').trim().split('\n').map((line) => JSON.parse(line));
        const rollingUploads = calls.filter((args) => args[0] === 'release' && args[1] === 'upload' && args[2] === 'nightly');
        if (channel === 'nightly') {
          expect(rollingUploads).toHaveLength(1);
          expect(rollingUploads[0][3]).toBe('output/release-assets/satchel.json');
          expect(JSON.parse(readFileSync(join(fixture.root, rollingUploads[0][3]), 'utf8'))).toEqual(manifest);
        } else {
          expect(rollingUploads).toHaveLength(0);
        }
        // Existing Daily clients still read nightly/latest.json; leave it alone.
        expect(rollingUploads.flat()).not.toContain('output/release-assets/latest.json');
      } finally {
        rmSync(fixture.root, { recursive: true, force: true });
      }
    });
  }

  it('does not publish or update a feed when a platform package is missing', () => {
    const fixture = releaseFixture('nightly');
    try {
      rmSync(join(fixture.root, 'output/release-assets/Satchel-linux-x64.AppImage'));
      expect(() => execFileSync(process.execPath, [publish], { cwd: fixture.root, env: fixture.env, stdio: 'pipe' })).toThrow();
      expect(readdirSync(fixture.root)).not.toContain('calls.jsonl');
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });
});
