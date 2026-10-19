import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const [plugin, command = 'handle-event', fixturePath] = process.argv.slice(2);
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pluginsRoot = join(repoRoot, 'plugins');

if (!plugin) {
  process.stderr.write('Usage: pnpm run plugin:simulate -- satchel-plugin-name [command] [fixture.json|inline-json]\n');
  process.exit(1);
}

const payload = fixturePath
  ? existsSync(fixturePath)
    ? readFileSync(fixturePath, 'utf8')
    : fixturePath
  : JSON.stringify({
      schema_version: 1,
      event: 'message.received',
      idempotency_key: 'simulated-message.received-1',
      payload: {
        account_id: 'acc-sim',
        thread_id: 'th-sim',
        from_address: 'sender@example.com',
        subject: 'Simulated event',
        preview: 'This is a local plugin simulator payload.',
      },
    });
/** @param {string} dir */
function packageName(dir) {
  const manifestPath = join(dir, 'Cargo.toml');
  if (!existsSync(manifestPath)) return null;
  return readFileSync(manifestPath, 'utf8').match(/^\s*name\s*=\s*"([^"]+)"/m)?.[1] ?? null;
}

const pluginDir = readdirSync(pluginsRoot, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => join(pluginsRoot, entry.name))
  .find((dir) => packageName(dir) === plugin || entryBasename(dir) === plugin);
const componentPath = pluginDir ? join(pluginDir, 'dist', 'component.wasm') : null;

if (!componentPath || !existsSync(componentPath)) {
  process.stderr.write(`No built component found for ${plugin}. Run pnpm run plugins:build:debug first.\n`);
  process.exit(1);
}

const result = spawnSync('cargo', [
  'run',
  '--quiet',
  '--manifest-path',
  join(repoRoot, 'src-tauri', 'Cargo.toml'),
  '--example',
  'plugin_sim',
  '--',
  componentPath,
  command,
  payload,
], {
  stdio: 'inherit',
});

process.exit(result.status ?? 1);

/** @param {string} dir */
function entryBasename(dir) {
  return dir.split(/[/\\]/).pop();
}
