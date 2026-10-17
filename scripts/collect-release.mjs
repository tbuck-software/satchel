// @ts-check
import { createHash } from 'node:crypto';
import { copyFileSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { PLATFORMS } from './release-model.mjs';

const target = /** @type {keyof typeof PLATFORMS} */ (process.argv[2]);
const platform = PLATFORMS[target];
if (!platform) throw new Error('Unknown release target');
const plan = JSON.parse(readFileSync('output/release-plan.json', 'utf8'));
const root = join('target', target, 'release', 'bundle');
const out = join('output', 'release-artifacts', target);
mkdirSync(out, { recursive: true });
/** @param {string} folder @returns {string[]} */
function files(folder) {
  return readdirSync(folder, { withFileTypes: true }).flatMap(entry => {
    const path = join(folder, entry.name);
    // Do not descend into app bundles, where unrelated files can share extensions.
    return entry.isDirectory() && !entry.name.endsWith('.app') ? files(path) : entry.isFile() ? [path] : [];
  });
}
const artifacts = files(root);
const extension = target.includes('windows') ? '.exe' : platform.updater;
const updater = artifacts.filter(file => file.endsWith(extension));
if (updater.length !== 1) throw new Error(`Expected one ${extension} updater for ${target}`);
const prefix = `Satchel${plan.channel === 'nightly' ? '-Daily' : ''}-${platform.name}`;
const updaterName = prefix + platform.updater;
const signature = readFileSync(updater[0] + '.sig', 'utf8').trim();
if (!signature) throw new Error('Missing updater signature');
copyFileSync(updater[0], join(out, updaterName));
copyFileSync(updater[0] + '.sig', join(out, updaterName + '.sig'));
if (platform.installer) {
  const installer = artifacts.filter(file => file.endsWith(platform.installer));
  if (installer.length !== 1) throw new Error(`Expected one ${platform.installer} installer`);
  copyFileSync(installer[0], join(out, prefix + platform.installer));
}
const checksums = readdirSync(out).map(name => `${createHash('sha256').update(readFileSync(join(out, name))).digest('hex')}  ${name}`).join('\n') + '\n';
writeFileSync(join(out, prefix + '-SHA256SUMS'), checksums);
writeFileSync(join(out, platform.platform + '.json'), JSON.stringify({ ...plan, platform: platform.platform, name: updaterName, signature }, null, 2) + '\n');
