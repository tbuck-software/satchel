// @ts-check
import { createHash, createPublicKey, timingSafeEqual, verify } from 'node:crypto';

export const RELEASE_REPOSITORY = 'tbuck-software/satchel';
export const PLATFORMS = /** @type {const} */ ({
  'aarch64-apple-darwin': { platform: 'darwin-aarch64', name: 'mac-arm64', updater: '.app.tar.gz', installer: '.dmg' },
  'x86_64-apple-darwin': { platform: 'darwin-x86_64', name: 'mac-x64', updater: '.app.tar.gz', installer: '.dmg' },
  'x86_64-unknown-linux-gnu': { platform: 'linux-x86_64', name: 'linux-x64', updater: '.AppImage', installer: /** @type {string | null} */ (null) },
  'x86_64-pc-windows-msvc': { platform: 'windows-x86_64', name: 'windows-x64', updater: '-setup.exe', installer: /** @type {string | null} */ (null) },
});

/** @param {string} value */
function stableVersion(value) {
  const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(value);
  if (!match) throw new Error(`Expected a stable semantic version, got ${value}`);
  return match.slice(1).map(Number);
}

/** @param {string} base @param {string | null} latest @param {'stable' | 'nightly'} channel @param {number} run */
export function releaseVersion(base, latest, channel, run) {
  const versions = [stableVersion(base), ...(latest ? [stableVersion(latest)] : [])];
  versions.sort((a, b) => (a[0] - b[0]) || (a[1] - b[1]) || (a[2] - b[2]));
  const [major, minor, patch] = versions.at(-1) ?? [0, 1, 0];
  if (!Number.isSafeInteger(run) || run < 1) throw new Error('A positive workflow run number is required');
  if (patch >= 65535) throw new Error('Bump the base minor version before the next Windows release');
  const next = `${major}.${minor}.${patch + 1}`;
  return channel === 'nightly' ? `${next}-nightly.${run}` : next;
}

/** Compare the stable/nightly version forms produced by this release workflow.
 * @param {string} left @param {string} right
 */
export function compareReleaseVersions(left, right) {
  const pattern = /^(\d+)\.(\d+)\.(\d+)(?:-nightly\.(\d+))?$/;
  const a = pattern.exec(left);
  const b = pattern.exec(right);
  if (!a || !b) throw new Error('Unrecognized installed release version');
  for (let index = 1; index <= 3; index += 1) {
    const difference = Number(a[index]) - Number(b[index]);
    if (difference) return Math.sign(difference);
  }
  if (!a[4] || !b[4]) return a[4] ? -1 : b[4] ? 1 : 0;
  return Math.sign(Number(a[4]) - Number(b[4]));
}

/** Verify Tauri/minisign signatures without trusting the downloaded manifest.
 * @param {Buffer} bytes @param {string} signature @param {string} publicKey
 */
export function verifyUpdate(bytes, signature, publicKey) {
  const keyLines = Buffer.from(publicKey, 'base64').toString('utf8').trim().split(/\r?\n/);
  const sigLines = Buffer.from(signature, 'base64').toString('utf8').trim().split(/\r?\n/);
  if (keyLines.length !== 2 || sigLines.length !== 4 || !sigLines[2].startsWith('trusted comment: ')) {
    throw new Error('Invalid update signing metadata');
  }
  const key = Buffer.from(keyLines[1], 'base64');
  const sig = Buffer.from(sigLines[1], 'base64');
  const globalSig = Buffer.from(sigLines[3], 'base64');
  if (key.length !== 42 || sig.length !== 74 || globalSig.length !== 64
    || key.subarray(0, 2).toString() !== 'Ed'
    || !timingSafeEqual(key.subarray(2, 10), sig.subarray(2, 10))) {
    throw new Error('Update signing key does not match');
  }
  const algorithm = sig.subarray(0, 2).toString();
  if (algorithm !== 'ED' && algorithm !== 'Ed') throw new Error('Unsupported update signature');
  const spki = Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key.subarray(10)]);
  const verifier = createPublicKey({ key: spki, format: 'der', type: 'spki' });
  const message = algorithm === 'ED' ? createHash('blake2b512').update(bytes).digest() : bytes;
  const trusted = Buffer.from(sigLines[2].slice('trusted comment: '.length));
  if (!verify(null, message, verifier, sig.subarray(10))
    || !verify(null, Buffer.concat([sig.subarray(10), trusted]), verifier, globalSig)) {
    throw new Error('Update signature verification failed');
  }
}
