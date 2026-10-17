// @ts-check
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';

const plan = JSON.parse(readFileSync('output/release-plan.json', 'utf8'));
if (!/^\d+\.\d+\.\d+(?:-nightly\.\d+)?$/.test(plan.version)) throw new Error('Invalid release version');
const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
pkg.version = plan.version;
writeFileSync('package.json', JSON.stringify(pkg, null, 2) + '\n');
for (const file of ['src-tauri/Cargo.toml', 'Cargo.lock']) {
  const text = readFileSync(file, 'utf8');
  const pattern = file.endsWith('Cargo.toml') ? /^version = "[^"]+"/m : /(\[\[package\]\]\nname = "satchel"\n)version = "[^"]+"/;
  if (!pattern.test(text)) throw new Error(`Cannot update version in ${file}`);
  writeFileSync(file, text.replace(pattern, file.endsWith('Cargo.toml') ? `version = "${plan.version}"` : `$1version = "${plan.version}"`));
}
const nightly = plan.channel === 'nightly';
const base = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const daily = JSON.parse(readFileSync('src-tauri/tauri.daily.conf.json', 'utf8'));
const config = {
  version: plan.version,
  ...(nightly ? { ...daily, app: { ...daily.app, windows: [{ ...base.app.windows[0], ...daily.app.windows[0] }] } } : {}),
  bundle: {
    createUpdaterArtifacts: true,
    resources: { '../output/release-notices/': 'licenses/', '../output/release-plan.json': 'release.json' },
    ...(process.platform === 'darwin' ? { macOS: { signingIdentity: process.env.APPLE_SIGNING_IDENTITY || '-' } } : {}),
  },
};
for (const plugin of readdirSync('plugins', { withFileTypes: true }).filter(entry => entry.isDirectory())) {
  for (const filename of ['Cargo.toml', 'dist/component.wasm']) {
    config.bundle.resources[`../plugins/${plugin.name}/${filename}`] = `plugins/${plugin.name}/${filename}`;
  }
}
writeFileSync('output/release-config.json', JSON.stringify(config, null, 2) + '\n');
