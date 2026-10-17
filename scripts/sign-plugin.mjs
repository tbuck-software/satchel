#!/usr/bin/env node
import { createHash, createPrivateKey, generateKeyPairSync, sign } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';

const artifactPath = process.argv[2];
if (!artifactPath) {
  console.error('Usage: pnpm run plugin:sign -- <path-to-component.wasm>');
  process.exit(1);
}

const keyPath = process.env.SATCHEL_PLUGIN_SIGNING_KEY
  ?? '.private/plugin-signing-key.pem';
if (!existsSync(keyPath)) {
  // First run on a fresh checkout: create the repository-local signer.
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  await writeFile(keyPath, privateKey.export({ type: 'pkcs8', format: 'pem' }), { mode: 0o600 });
  const rawPublic = publicKey.export({ type: 'spki', format: 'der' }).subarray(-32);
  console.error(
    `Generated new plugin signing key at ${keyPath}.` +
    ` Trust its public key by adding "${rawPublic.toString('hex')}"` +
    ` to SATCHEL_PLUGIN_TRUSTED_KEYS in src-tauri/src/plugins/registry.rs.`,
  );
}

const bytes = await readFile(artifactPath);
const signature = sign(null, bytes, createPrivateKey(await readFile(keyPath, 'utf8')));
const digest = createHash('sha256').update(bytes).digest('hex');

console.log(`ed25519:${signature.toString('hex')}`);
console.error(`sha256 (integrity reference only): sha256:${digest}`);
