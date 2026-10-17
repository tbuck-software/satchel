import { cpSync, existsSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pluginsRoot = join(repoRoot, 'plugins');
const [command, arg] = process.argv.slice(2);

function readPackageName(manifestPath) {
  const manifest = readFileSync(manifestPath, 'utf8');
  return manifest.match(/^\s*name\s*=\s*"([^"]+)"/m)?.[1] ?? null;
}

function pluginDirs() {
  return readdirSync(pluginsRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => join(pluginsRoot, entry.name))
    .filter((dir) => existsSync(join(dir, 'Cargo.toml')));
}

function list() {
  for (const dir of pluginDirs()) {
    const manifestPath = join(dir, 'Cargo.toml');
    const name = readPackageName(manifestPath) ?? dir;
    const hasComponent = existsSync(join(dir, 'dist', 'component.wasm'));
    process.stdout.write(`${name}\t${hasComponent ? 'built' : 'source-only'}\t${dir}\n`);
  }
}

function install(source) {
  if (!source) {
    process.stderr.write('Usage: pnpm run plugin -- install /path/to/plugin\n');
    process.exit(1);
  }
  const sourceDir = resolve(source);
  const manifestPath = join(sourceDir, 'Cargo.toml');
  if (!existsSync(manifestPath)) {
    process.stderr.write(`No Cargo.toml found in ${sourceDir}\n`);
    process.exit(1);
  }
  const packageName = readPackageName(manifestPath);
  if (!packageName) {
    process.stderr.write(`Could not read package name from ${manifestPath}\n`);
    process.exit(1);
  }
  const targetName = packageName.replace(/^satchel-plugin-/, '');
  const targetDir = join(pluginsRoot, targetName);
  rmSync(targetDir, { recursive: true, force: true });
  cpSync(sourceDir, targetDir, {
    recursive: true,
    filter: (path) => !path.includes(`${sourceDir}/target`),
  });
  process.stdout.write(`Installed ${packageName} -> ${targetDir}\n`);
}

function remove(name) {
  if (!name) {
    process.stderr.write('Usage: pnpm run plugin -- remove plugin-name\n');
    process.exit(1);
  }
  const dir = join(pluginsRoot, name.replace(/^satchel-plugin-/, ''));
  rmSync(dir, { recursive: true, force: true });
  process.stdout.write(`Removed ${dir}\n`);
}

switch (command) {
  case 'list':
    list();
    break;
  case 'install':
  case 'update':
    install(arg);
    break;
  case 'remove':
    remove(arg);
    break;
  default:
    process.stderr.write('Usage: pnpm run plugin -- list|install|update|remove [path|name]\n');
    process.exit(1);
}
