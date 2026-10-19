import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';

const OAUTH_KEYS = [
  'SATCHEL_GMAIL_CLIENT_ID',
  'SATCHEL_GMAIL_CLIENT_SECRET',
  'SATCHEL_GMAIL_REDIRECT_URI',
  'SATCHEL_OUTLOOK_CLIENT_ID',
  'SATCHEL_OUTLOOK_CLIENT_SECRET',
  'SATCHEL_OUTLOOK_REDIRECT_URI',
];

/**
 * @param {string} repoRoot
 */
function oauthEnvFile(repoRoot) {
  const local = join(repoRoot, '.env');
  if (existsSync(local)) return local;
  try {
    const pointer = readFileSync(join(repoRoot, '.git'), 'utf8');
    if (!pointer.startsWith('gitdir:')) return local;
    const gitdir = resolve(repoRoot, pointer.slice('gitdir:'.length).trim());
    const worktrees = dirname(gitdir);
    if (basename(worktrees) === 'worktrees') return join(dirname(dirname(worktrees)), '.env');
  } catch {
    // Source archives and ordinary checkouts need no worktree fallback.
  }
  return local;
}

/** Only personal debug installs receive runtime credentials, never APKs. */
/**
 * @param {string} adb
 * @param {string} serial
 * @param {string} packageId
 * @param {string} repoRoot
 */
export function provisionAndroidOAuthConfig(adb, serial, packageId, repoRoot) {
  const file = oauthEnvFile(repoRoot);
  const contents = existsSync(file) ? readFileSync(file, 'utf8') : '';
  const values = new Map();
  for (const line of contents.split(/\r?\n/)) {
    const match = line.trim().match(/^([A-Z_]+)\s*=\s*(.*)$/);
    if (!match || !OAUTH_KEYS.includes(match[1])) continue;
    const value = match[2].trim().replace(/^(['"])(.*)\1$/, '$2');
    if (!values.has(match[1])) values.set(match[1], value);
  }
  const payload = OAUTH_KEYS.flatMap((key) => {
    const value = process.env[key] ?? values.get(key);
    if (!value?.trim() || /[\r\n]/.test(value)) return [];
    return [`${key}=${JSON.stringify(value.trim())}\n`];
  }).join('');
  if (!payload) return false;
  if (!/^[a-zA-Z0-9_.]+$/.test(packageId)) throw new Error('Invalid Android package identifier.');

  // Credentials travel only over stdin to an app-owned, mode-600 file. The
  // existing load_app_support_env reads this path at the next process start.
  const result = spawnSync(adb, [
    '-s', serial, 'shell', '-T', 'run-as', packageId, 'sh', '-c',
    "'umask 077; mkdir -p satchel && cat > satchel/.env.pending && chmod 600 satchel/.env.pending && mv satchel/.env.pending satchel/.env'",
  ], { cwd: repoRoot, input: payload, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] });
  if (result.status !== 0) {
    throw new Error('Could not provision OAuth configuration in private Android app storage.');
  }
  return true;
}
