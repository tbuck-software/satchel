// @ts-check
import { createPrivateKey, createPublicKey, sign } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const pem = process.env.SATCHEL_PLUGIN_SIGNING_PRIVATE_KEY
  ?? (process.env.SATCHEL_PLUGIN_SIGNING_KEY ? readFileSync(process.env.SATCHEL_PLUGIN_SIGNING_KEY, 'utf8') : null);
if (!pem) throw new Error('Release plugins require SATCHEL_PLUGIN_SIGNING_PRIVATE_KEY or SATCHEL_PLUGIN_SIGNING_KEY');
const key = createPrivateKey(pem);
if (key.asymmetricKeyType !== 'ed25519') throw new Error('Plugin signing requires an Ed25519 key');
const publicHex = createPublicKey(key).export({ type: 'spki', format: 'der' }).subarray(-32).toString('hex');
const registry = readFileSync('src-tauri/src/plugins/registry.rs', 'utf8');
const trusted = /const SATCHEL_PLUGIN_TRUSTED_KEYS:[\s\S]*?=\s*&\[([^\]]*)\]/.exec(registry)?.[1];
if (!trusted?.includes(`"${publicHex}"`)) throw new Error('Plugin signing key is not trusted by the release host');

const plugins = readdirSync('plugins', { withFileTypes: true })
  .filter(entry => entry.isDirectory() && existsSync(join('plugins', entry.name, 'Cargo.toml')))
  .map(entry => {
    const source = join('plugins', entry.name);
    const manifest = readFileSync(join(source, 'Cargo.toml'), 'utf8').replace(/\r\n/g, '\n');
    const bytes = readFileSync(join(source, 'dist/component.wasm'));
    if (!bytes.subarray(0, 8).equals(Buffer.from([0, 97, 115, 109, 13, 0, 1, 0]))) {
      throw new Error(`Not a built WASI component: ${entry.name}`);
    }
    const header = '[package.metadata.satchel]\n';
    const start = manifest.indexOf(header);
    if (start < 0) throw new Error(`Plugin metadata missing: ${entry.name}`);
    const tail = manifest.slice(start + header.length);
    const boundary = tail.search(/^\[/m);
    const metadata = boundary < 0 ? tail : tail.slice(0, boundary);
    const remaining = boundary < 0 ? '' : tail.slice(boundary);
    const fields = metadata.replace(/^(?:trust|signature)\s*=.*\n?/gm, '').trimEnd();
    const signature = sign(null, bytes, key).toString('hex');
    const signedManifest = manifest.slice(0, start) + header + fields
      + `\ntrust = "verified"\nsignature = "ed25519:${signature}"\n\n` + remaining;
    return { name: entry.name, bytes, manifest: signedManifest };
  });
if (plugins.length === 0) throw new Error('No release plugins found');

// Keep development manifests unsigned. Only this generated copy enters a release.
const output = 'output/bundled-plugins';
rmSync(output, { recursive: true, force: true });
for (const plugin of plugins) {
  const destination = join(output, plugin.name);
  mkdirSync(join(destination, 'dist'), { recursive: true });
  writeFileSync(join(destination, 'Cargo.toml'), plugin.manifest);
  writeFileSync(join(destination, 'dist/component.wasm'), plugin.bytes);
  console.log(`Signed release plugin: ${plugin.name}`);
}
