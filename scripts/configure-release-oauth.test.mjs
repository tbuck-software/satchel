import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const script = fileURLToPath(new URL('./configure-release-oauth.mjs', import.meta.url));
const buildScript = fileURLToPath(new URL('../crates/satchel-core/build.rs', import.meta.url));
const installed = { client_id: 'fixture.apps.googleusercontent.com', client_secret: 'desktop-fixture', token_uri: 'https://oauth2.googleapis.com/token' };

describe('Google desktop release configuration', () => {
  it('carries the validated desktop credential through the actual Rust build script', () => {
    const root = mkdtempSync(join(tmpdir(), 'satchel-oauth-build-'));
    try {
      const output = join(root, 'github-env');
      execFileSync(process.execPath, [script], {
        env: { ...process.env, GITHUB_ENV: output, SATCHEL_GMAIL_DESKTOP_CLIENT_CONFIG: JSON.stringify({ installed }) },
      });
      const values = Object.fromEntries(readFileSync(output, 'utf8').trim().split('\n').map(line => {
        const index = line.indexOf('=');
        return [line.slice(0, index), line.slice(index + 1)];
      }));
      expect(values.SATCHEL_GMAIL_CLIENT_ID).toBe(installed.client_id);
      expect(values.SATCHEL_GMAIL_DESKTOP_CLIENT_SECRET).toBe(installed.client_secret);
      const binary = join(root, process.platform === 'win32' ? 'oauth-build.exe' : 'oauth-build');
      execFileSync('rustc', [buildScript, '-o', binary]);
      const result = execFileSync(binary, [], {
        cwd: root,
        env: { ...process.env, ...values, CARGO_MANIFEST_DIR: root, CARGO_CFG_TARGET_OS: 'macos' },
        encoding: 'utf8',
      });
      expect(result).toContain('cargo:rustc-env=SATCHEL_GMAIL_CLIENT_ID=fixture.apps.googleusercontent.com');
      expect(result).toContain('cargo:rustc-env=SATCHEL_GMAIL_DESKTOP_CLIENT_SECRET=desktop-fixture');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  for (const [reason, config] of [
    ['web clients', { web: installed }],
    ['ambiguous clients', { installed, web: installed }],
    ['missing configuration', null],
    ['missing desktop credential', { installed: { ...installed, client_secret: '' } }],
    ['untrusted token endpoints', { installed: { ...installed, token_uri: 'https://example.com/token' } }],
    ['multiline values', { installed: { ...installed, client_secret: 'desktop-fixture\nEXTRA=bad' } }],
  ]) {
    it(`rejects ${reason} before writing build inputs`, () => {
      const root = mkdtempSync(join(tmpdir(), 'satchel-oauth-config-'));
      try {
        const output = join(root, 'github-env');
        const result = spawnSync(process.execPath, [script], {
          env: { ...process.env, GITHUB_ENV: output, SATCHEL_GMAIL_DESKTOP_CLIENT_CONFIG: config ? JSON.stringify(config) : '' },
          encoding: 'utf8',
        });
        expect(result.status).not.toBe(0);
        expect(result.stderr).not.toContain('desktop-fixture');
        expect(() => readFileSync(output)).toThrow();
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    });
  }
});
