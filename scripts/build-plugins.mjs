import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pluginsRoot = join(repoRoot, 'plugins');
const release = process.argv.includes('--release');
const profile = release ? 'release' : 'debug';
const wasmTarget = 'wasm32-wasip2';

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    ...options,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function parsePackageName(manifestPath) {
  const manifest = readFileSync(manifestPath, 'utf8');
  const match = manifest.match(/^\s*name\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error(`Could not parse package name from ${manifestPath}`);
  }
  return match[1];
}

function ensureWasmTarget() {
  const installed = spawnSync('rustup', ['target', 'list', '--installed'], {
    cwd: repoRoot,
    encoding: 'utf8',
  });
  if (installed.status !== 0) {
    process.stderr.write(installed.stderr || 'Failed to inspect installed Rust targets.\n');
    process.exit(installed.status ?? 1);
  }

  if (!installed.stdout.split(/\r?\n/).includes(wasmTarget)) {
    run('rustup', ['target', 'add', wasmTarget]);
  }
}

function discoverPlugins() {
  return readdirSync(pluginsRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => {
      const rootDir = join(pluginsRoot, entry.name);
      const manifestPath = join(rootDir, 'Cargo.toml');
      if (!existsSync(manifestPath)) {
        return null;
      }
      const packageName = parsePackageName(manifestPath);
      return {
        id: packageName,
        rootDir,
        manifestPath,
        crateName: packageName.replace(/-/g, '_'),
      };
    })
    .filter(Boolean);
}

function buildPlugin(plugin) {
  const targetDir = join(plugin.rootDir, 'target');
  const distDir = join(plugin.rootDir, 'dist');
  const sourceWasm = join(targetDir, wasmTarget, profile, `${plugin.crateName}.wasm`);
  const componentWasm = join(distDir, 'component.wasm');

  rmSync(distDir, { recursive: true, force: true });
  mkdirSync(distDir, { recursive: true });

  run(
    'cargo',
    [
      'build',
      '--target',
      wasmTarget,
      '--manifest-path',
      plugin.manifestPath,
      ...(release ? ['--release'] : []),
    ],
    {
      env: {
        ...process.env,
        CARGO_TARGET_DIR: targetDir,
      },
    },
  );

  if (!existsSync(sourceWasm)) {
    process.stderr.write(`Expected component artifact missing: ${sourceWasm}\n`);
    process.exit(1);
  }

  copyFileSync(sourceWasm, componentWasm);
  process.stdout.write(`Built ${plugin.id} -> ${componentWasm}\n`);
}

ensureWasmTarget();
for (const plugin of discoverPlugins()) {
  buildPlugin(plugin);
}
