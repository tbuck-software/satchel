// @ts-check
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';

const out = 'output/release-notices';
mkdirSync(out, { recursive: true });
for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) copyFileSync(name, join(out, name));
for (const name of ['figtree', 'firacode']) copyFileSync(`public/fonts/OFL-${name}.txt`, join(out, `OFL-${name}.txt`));
copyFileSync('public/avatars/LICENSE.simple-icons.md', join(out, 'CC0-simple-icons.txt'));
copyFileSync('src/utils/data/UNICODE-LICENSE.txt', join(out, 'UNICODE-LICENSE.txt'));

/** @param {string} directory @param {string | null} explicit @returns {string[]} */
function notices(directory, explicit = null) {
  const candidates = readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isFile() && /^(licen[cs]e|copying|notice|copyright)([._-]|$)/i.test(entry.name))
    .map(entry => join(directory, entry.name));
  if (explicit && existsSync(explicit)) candidates.push(explicit);
  return [...new Set(candidates)].map(file => `${basename(file)}\n${readFileSync(file, 'utf8').trim()}`);
}

const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--format-version', '1'], { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }));
/** @type {string[]} */
const rust = [];
for (const pkg of metadata.packages) {
  if (metadata.workspace_members.includes(pkg.id)) continue;
  const texts = notices(dirname(pkg.manifest_path), pkg.license_file ?? null);
  rust.push(`${pkg.name} ${pkg.version}\nDeclared license: ${pkg.license ?? 'see upstream notice'}\n${texts.join('\n\n')}`);
}
writeFileSync(join(out, 'RUST-DEPENDENCIES.txt'), rust.sort().join('\n\n' + '='.repeat(72) + '\n\n') + '\n');

const licenses = JSON.parse(execFileSync(process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm', ['licenses', 'list', '--prod', '--json'], {
  encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, shell: process.platform === 'win32',
}));
/** @type {string[]} */
const javascript = [];
for (const entries of Object.values(licenses)) {
  for (const pkg of entries) {
    for (let index = 0; index < pkg.paths.length; index += 1) {
      const texts = notices(pkg.paths[index]);
      javascript.push(`${pkg.name} ${pkg.versions[index] ?? pkg.versions.join(', ')}\nDeclared license: ${pkg.license}\n${texts.join('\n\n')}`);
    }
  }
}
writeFileSync(join(out, 'JAVASCRIPT-DEPENDENCIES.txt'), javascript.sort().join('\n\n' + '='.repeat(72) + '\n\n') + '\n');
console.log(`Included notices for ${rust.length} Rust and ${javascript.length} JavaScript dependency packages.`);
