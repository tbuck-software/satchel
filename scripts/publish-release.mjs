// @ts-check
import { execFileSync } from 'node:child_process';
import { copyFileSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { PLATFORMS, RELEASE_REPOSITORY } from './release-model.mjs';
import { requireReleaseTag } from './release-tags.mjs';

if (process.env.GITHUB_REPOSITORY !== RELEASE_REPOSITORY || process.env.GITHUB_REF !== 'refs/heads/main') {
  throw new Error('Only the upstream main workflow may publish releases');
}
const plan = JSON.parse(readFileSync('output/release-plan.json', 'utf8'));
const folder = 'output/release-assets';
/** @type {Record<string, {signature: string, url: string}>} */
const platforms = {};
for (const platform of Object.values(PLATFORMS)) {
  const metadata = JSON.parse(readFileSync(join(folder, platform.platform + '.json'), 'utf8'));
  if (metadata.version !== plan.version || metadata.source !== plan.source || metadata.channel !== plan.channel) {
    throw new Error(`Mismatched release artifact for ${platform.platform}`);
  }
  // Reading the binary and signature here makes incomplete matrix uploads fail before publication.
  readFileSync(join(folder, metadata.name));
  if (readFileSync(join(folder, metadata.name + '.sig'), 'utf8').trim() !== metadata.signature) throw new Error('Mismatched updater signature');
  platforms[platform.platform] = {
    signature: metadata.signature,
    url: `https://github.com/${RELEASE_REPOSITORY}/releases/download/${plan.tag}/${metadata.name}`,
  };
}
const manifest = { version: plan.version, source: plan.source, pub_date: plan.published, notes: `Satchel ${plan.channel} from main.`, release_tag: plan.tag, channel: plan.channel, platforms };
writeFileSync(join(folder, 'latest.json'), JSON.stringify(manifest, null, 2) + '\n');
copyFileSync('output/installer/install-satchel.mjs', join(folder, 'install-satchel.mjs'));
for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) copyFileSync(name, join(folder, name));
const notes = `Satchel ${plan.channel} ${plan.version}\n\nBuilt from main at ${plan.source}.\n\n${plan.channel === 'nightly' ? 'Nightly builds may contain unfinished changes. They use the separate Satchel Daily profile.' : 'Stable desktop release.'}\n\n<!-- satchel-source:${plan.source} -->\n`;
writeFileSync('output/release-notes.md', notes);
/** @param {string[]} args @returns {string} */
function gh(args) {
  return execFileSync('gh', [...args, '--repo', RELEASE_REPOSITORY], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
}

// Publish only after every desktop platform and the verification job succeeded.
requireReleaseTag(plan.tag, plan.source);
gh(['release', 'create', plan.tag, '--verify-tag', '--title', `Satchel ${plan.version}`, '--draft', '--notes-file', 'output/release-notes.md', ...(plan.channel === 'nightly' ? ['--prerelease'] : [])]);
const assets = readdirSync(folder).filter(name => !Object.values(PLATFORMS).some(platform => name === platform.platform + '.json'));
gh(['release', 'upload', plan.tag, ...assets.map(name => join(folder, name))]);
gh(['release', 'edit', plan.tag, '--draft=false', ...(plan.channel === 'stable' ? ['--latest'] : [])]);
if (plan.channel === 'nightly') {
  let exists = true;
  try {
    execFileSync('gh', ['release', 'view', 'nightly', '--repo', RELEASE_REPOSITORY], { stdio: 'ignore' });
  } catch { exists = false; }
  if (!exists) gh(['release', 'create', 'nightly', '--verify-tag', '--title', 'Satchel Nightly', '--prerelease', '--notes-file', 'output/release-notes.md']);
  // Expose the channel manifest only after the immutable, signed versioned assets exist.
  gh(['release', 'upload', 'nightly', join(folder, 'latest.json'), '--clobber']);
  gh(['release', 'edit', 'nightly', '--notes-file', 'output/release-notes.md']);
}
console.log(`Published ${plan.tag} to the ${plan.channel} channel.`);
