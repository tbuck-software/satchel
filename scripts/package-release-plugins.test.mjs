import { generateKeyPairSync, verify } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const script = fileURLToPath(new URL('./package-release-plugins.mjs', import.meta.url));
const manifest = '[package]\nname = "satchel-plugin-calendar-sync"\nversion = "0.1.0"\n\n[package.metadata.satchel]\napi_version = "1.0"\ncapabilities = ["read_threads"]\ntrust = "local_unsigned"\n\n[lib]\ncrate-type = ["cdylib"]\n';
const component = Buffer.from([0, 97, 115, 109, 13, 0, 1, 0]);

/** @param {(context: {root: string, key: import('node:crypto').KeyPairKeyObjectResult, execute: (value?: string) => Buffer}) => void} run */
function fixture(run) {
  const root = mkdtempSync(join(tmpdir(), 'satchel-signed-plugins-'));
  const key = generateKeyPairSync('ed25519');
  const publicHex = key.publicKey.export({ type: 'spki', format: 'der' }).subarray(-32).toString('hex');
  const pem = String(key.privateKey.export({ type: 'pkcs8', format: 'pem' }));
  mkdirSync(join(root, 'plugins/calendar-sync/dist'), { recursive: true });
  mkdirSync(join(root, 'src-tauri/src/plugins'), { recursive: true });
  writeFileSync(join(root, 'plugins/calendar-sync/Cargo.toml'), manifest);
  writeFileSync(join(root, 'plugins/calendar-sync/dist/component.wasm'), component);
  writeFileSync(join(root, 'src-tauri/src/plugins/registry.rs'), `const SATCHEL_PLUGIN_TRUSTED_KEYS: &[&str] = &["${publicHex}"];`);
  const execute = (value = pem) => execFileSync(process.execPath, [script], {
    cwd: root,
    env: { ...process.env, SATCHEL_PLUGIN_SIGNING_PRIVATE_KEY: value, SATCHEL_PLUGIN_SIGNING_KEY: '' },
    stdio: 'pipe',
  });
  try { run({ root, key, execute }); } finally { rmSync(root, { recursive: true, force: true }); }
}

describe('signed bundled release plugins', () => {
  it('signs the exact shipped component and keeps source manifests unchanged', () => fixture(({ root, key, execute }) => {
    execute();
    const packaged = join(root, 'output/bundled-plugins/calendar-sync');
    const signedManifest = readFileSync(join(packaged, 'Cargo.toml'), 'utf8');
    const signature = /^signature = "ed25519:([a-f0-9]+)"$/m.exec(signedManifest)?.[1];
    if (!signature) throw new Error('Packaged plugin has no signature');
    expect(signedManifest).toContain('trust = "verified"');
    expect(signedManifest).toContain('[lib]\ncrate-type = ["cdylib"]');
    const bytes = readFileSync(join(packaged, 'dist/component.wasm'));
    expect(verify(null, bytes, key.publicKey, Buffer.from(signature, 'hex'))).toBe(true);
    expect(verify(null, Buffer.concat([bytes, Buffer.from('changed')]), key.publicKey, Buffer.from(signature, 'hex'))).toBe(false);
    expect(readFileSync(join(root, 'plugins/calendar-sync/Cargo.toml'), 'utf8')).toBe(manifest);
  }));

  it('refuses a missing key instead of shipping unsigned plugins', () => fixture(({ root, execute }) => {
    expect(() => execute('')).toThrow();
    expect(existsSync(join(root, 'output/bundled-plugins'))).toBe(false);
  }));

  it('refuses a signing key the host does not trust', () => fixture(({ root, execute }) => {
    const wrong = String(generateKeyPairSync('ed25519').privateKey.export({ type: 'pkcs8', format: 'pem' }));
    expect(() => execute(wrong)).toThrow();
    expect(existsSync(join(root, 'output/bundled-plugins'))).toBe(false);
  }));

  it('refuses missing or invalid WASM components', () => fixture(({ root, execute }) => {
    const artifact = join(root, 'plugins/calendar-sync/dist/component.wasm');
    rmSync(artifact);
    expect(() => execute()).toThrow();
    writeFileSync(artifact, 'not a component');
    expect(() => execute()).toThrow();
    expect(existsSync(join(root, 'output/bundled-plugins'))).toBe(false);
  }));
});
