// @ts-check
import { execFileSync } from 'node:child_process';
import { RELEASE_REPOSITORY, releaseVersion } from './release-model.mjs';

/** @typedef {{read: (tag: string) => string | null, create: (tag: string, source: string) => void}} ReleaseTags */
/** @typedef {{type: string, sha: string}} GitObject */

/** @param {string[]} args @returns {unknown} */
function api(args) {
  return JSON.parse(execFileSync('gh', ['api', ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }));
}

/** @type {ReleaseTags} */
const githubTags = {
  read(tag) {
    const refs = /** @type {{ref: string, object: GitObject}[]} */ (api([
      `repos/${RELEASE_REPOSITORY}/git/matching-refs/tags/${encodeURIComponent(tag)}`,
    ]));
    const found = refs.find(ref => ref.ref === `refs/tags/${tag}`);
    if (!found) return null;
    let object = found.object;
    // Existing manually created tags may be annotated rather than lightweight.
    for (let depth = 0; object.type === 'tag' && depth < 5; depth += 1) {
      object = /** @type {{object: GitObject}} */ (api([`repos/${RELEASE_REPOSITORY}/git/tags/${object.sha}`])).object;
    }
    if (object.type !== 'commit' || !/^[a-f0-9]{40}$/.test(object.sha)) throw new Error('Release tag does not resolve to a commit');
    return object.sha;
  },
  create(tag, source) {
    const result = /** @type {{object?: GitObject}} */ (api(['--method', 'POST', `repos/${RELEASE_REPOSITORY}/git/refs`,
      '-f', `ref=refs/tags/${tag}`, '-f', `sha=${source}`]));
    if (result.object?.type !== 'commit' || result.object.sha !== source) throw new Error('Created release tag points at another commit');
  },
};

/** Reserve immutable source tags before the long build, while the commit is current main.
 * @param {{version: string, channel: 'stable' | 'nightly', source: string}} plan
 * @param {ReleaseTags} tags
 */
export function reserveReleaseTags(plan, tags = githubTags) {
  let version = plan.version;
  for (let attempts = 0; attempts < 100; attempts += 1) {
    const tag = `v${version}`;
    const existing = tags.read(tag);
    if (existing === null || existing === plan.source) {
      if (existing === null) tags.create(tag, plan.source);
      if (plan.channel === 'nightly' && tags.read('nightly') === null) tags.create('nightly', plan.source);
      return { version, tag };
    }
    if (plan.channel === 'nightly') throw new Error('This nightly tag belongs to another commit; dispatch a new run');
    // A failed build can leave its reserved stable tag. Never move that tag.
    version = releaseVersion(version, null, 'stable', 1);
  }
  throw new Error('Too many reserved stable versions without a published release');
}

/** @param {string} tag @param {string} source @param {ReleaseTags} tags */
export function requireReleaseTag(tag, source, tags = githubTags) {
  if (tags.read(tag) !== source) throw new Error('The release tag does not match the verified build');
}
