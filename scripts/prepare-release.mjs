// @ts-check
import { execFileSync } from 'node:child_process';
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { RELEASE_REPOSITORY, releaseVersion } from './release-model.mjs';
import { reserveReleaseTags } from './release-tags.mjs';

const channel = process.env.RELEASE_CHANNEL;
if (channel !== 'stable' && channel !== 'nightly') throw new Error('Expected stable or nightly channel');
if (process.env.GITHUB_REPOSITORY !== RELEASE_REPOSITORY || process.env.GITHUB_REF !== 'refs/heads/main') {
  throw new Error('Releases must build main of the upstream repository');
}
/** @param {string} path */
function readRelease(path) {
  try {
    return JSON.parse(execFileSync('gh', ['api', `repos/${RELEASE_REPOSITORY}/releases/${path}`], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }));
  } catch (error) {
    const detail = /** @type {{stderr?: Buffer | string}} */ (error).stderr?.toString() ?? '';
    if (detail.includes('404')) return null;
    throw new Error('Could not inspect the previous release');
  }
}

const source = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const stable = readRelease('latest');
const previous = channel === 'nightly' ? readRelease('tags/nightly') : stable;
const previousSource = /<!-- satchel-source:([a-f0-9]{40}) -->/.exec(previous?.body ?? '')?.[1];
const build = process.env.RELEASE_FORCE === 'true' || previousSource !== source;
if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `build=${build}\n`);
if (!build) {
  console.log('main has not changed since the previous release on this channel.');
} else {
  const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
  const version = releaseVersion(pkg.version, stable?.tag_name ?? null, channel, Number(process.env.GITHUB_RUN_NUMBER));
  const reserved = reserveReleaseTags({ channel, version, source });
  const plan = { channel, ...reserved, source, published: new Date().toISOString() };
  mkdirSync('output', { recursive: true });
  writeFileSync('output/release-plan.json', JSON.stringify(plan, null, 2) + '\n');
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `version=${plan.version}\n`);
  console.log(`Prepared ${channel} ${plan.version} from ${source}`);
}
