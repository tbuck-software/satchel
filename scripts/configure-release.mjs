// @ts-check
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';

const plan = JSON.parse(readFileSync('output/release-plan.json', 'utf8'));
if (!/^\d+\.\d+\.\d+(?:-nightly\.\d+)?$/.test(plan.version)) throw new Error('Invalid release version');
const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
pkg.version = plan.version;
writeFileSync('package.json', JSON.stringify(pkg, null, 2) + '\n');
for (const file of ['src-tauri/Cargo.toml', 'Cargo.lock']) {
  const text = readFileSync(file, 'utf8');
  const pattern = file.endsWith('Cargo.toml') ? /^version = "[^"]+"/m : /(\[\[package\]\]\r?\nname = "satchel"\r?\n)version = "[^"]+"/;
  if (!pattern.test(text)) throw new Error(`Cannot update version in ${file}`);
  writeFileSync(file, text.replace(pattern, file.endsWith('Cargo.toml') ? `version = "${plan.version}"` : `$1version = "${plan.version}"`));
}
/** @type {Record<string, string>} */
const resources = { '../output/release-notices/': 'licenses/', '../output/release-plan.json': 'release.json' };
const config = {
  version: plan.version,
  // Update channels share the regular app's bundle identity and data profile.
  // The Daily config remains available only for isolated local source builds.
  productName: 'Satchel',
  identifier: 'de.tbuck.satchel',
  bundle: {
    createUpdaterArtifacts: true,
    resources,
    ...(process.platform === 'darwin' ? { macOS: { signingIdentity: process.env.APPLE_SIGNING_IDENTITY || '-' } } : {}),
  },
};
for (const plugin of readdirSync('plugins', { withFileTypes: true }).filter(entry => entry.isDirectory())) {
  for (const filename of ['Cargo.toml', 'dist/component.wasm']) {
    config.bundle.resources[`../output/bundled-plugins/${plugin.name}/${filename}`] = `plugins/${plugin.name}/${filename}`;
  }
}
writeFileSync('output/release-config.json', JSON.stringify(config, null, 2) + '\n');
