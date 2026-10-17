import ts from 'typescript';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readdirSync, readFileSync, rmSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const destination = join(root, 'src/services/backend/generated');
const staging = mkdtempSync(join(tmpdir(), 'satchel-ipc-'));
const check = process.argv.includes('--check');

/** @param {string[]} args */
function run(args) {
  const result = spawnSync('cargo', args, {
    cwd: root,
    stdio: 'inherit',
    env: { ...process.env, TS_RS_EXPORT_DIR: staging, TS_RS_LARGE_INT: 'number' },
  });
  if (result.status !== 0) throw new Error(`cargo ${args.join(' ')} failed`);
}

/**
 * @param {string} directory
 * @param {string} [prefix]
 * @returns {string[]}
 */
function tree(directory, prefix = '') {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const name = join(prefix, entry.name);
    return entry.isDirectory() ? tree(join(directory, entry.name), name) : [name];
  }).sort();
}

try {
  run(['test', '--locked', '-p', 'satchel-ipc-manifest', '--quiet']);
  run(['run', '--locked', '-p', 'satchel-ipc-manifest', '--', staging]);
  run(['test', '--locked', '--quiet', '-p', 'satchel', '-p', 'satchel-core', '--features', 'ipc-types', '--lib', 'export_bindings', '--', '--test-threads=1']);
  const generated = tree(staging);
  // ts-rs includes its own banner; keep the command for refreshing nearby.
  for (const path of generated) {
    const file = join(staging, path);
    const source = readFileSync(file, 'utf8').replace(/^\/\/ This file was generated[^\n]*\n/, '// Generated from Rust. Run pnpm ipc:generate.\n');
    const parsed = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true);
    writeFileSync(file, ts.createPrinter({ newLine: ts.NewLineKind.LineFeed }).printFile(parsed));
  }
  if (check) {
    const expected = tree(destination);
    const changed = [...new Set([...expected, ...generated])].filter((path) => {
      if (!expected.includes(path) || !generated.includes(path)) return true;
      return readFileSync(join(destination, path), 'utf8') !== readFileSync(join(staging, path), 'utf8');
    });
    if (changed.length) throw new Error(`IPC bindings are stale. Run pnpm ipc:generate:\n${changed.join('\n')}`);
    console.info(`IPC bindings match Rust (${generated.length} files).`);
  } else {
    rmSync(destination, { recursive: true, force: true });
    for (const path of generated) {
      mkdirSync(dirname(join(destination, path)), { recursive: true });
      writeFileSync(join(destination, path), readFileSync(join(staging, path)));
    }
    console.info(`Wrote ${generated.length} IPC binding files.`);
  }
} finally {
  rmSync(staging, { recursive: true, force: true });
}
