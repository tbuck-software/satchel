import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

/** @type {Record<string, { reason: string, diagnostics: Record<string, number> }>} */
const allowed = JSON.parse(readFileSync(new URL('./implicit-any-allowlist.json', import.meta.url), 'utf8'));
const require = createRequire(import.meta.url);
const result = spawnSync(process.execPath, [require.resolve('typescript/bin/tsc'), '-p', 'tsconfig.scripts.json', '--noImplicitAny', '--pretty', 'false'], {
  encoding: 'utf8',
});
if (result.error || result.status === null) throw result.error ?? new Error('TypeScript did not exit normally');
const output = result.stdout + result.stderr;
/** @type {Record<string, Record<string, number>>} */
const actual = {};
for (const line of output.split('\n').filter((line) => line.trim())) {
  const match = /^(scripts\/[^()]+)\(\d+,\d+\): error TS(\d+):/.exec(line);
  if (!match || !['7005', '7006', '7018', '7019', '7023', '7024', '7031', '7034', '7053'].includes(match[2])) {
    throw new Error(`Unexpected TypeScript diagnostic: ${line}`);
  }
  const file = actual[match[1]] ??= {};
  file[match[2]] = (file[match[2]] ?? 0) + 1;
}
if (result.status !== 0 && !Object.keys(actual).length) throw new Error('TypeScript failed without diagnostics');
let failed = false;
for (const file of new Set([...Object.keys(allowed), ...Object.keys(actual)])) {
  const expected = allowed[file]?.diagnostics ?? {};
  if (allowed[file] && !allowed[file].reason.trim()) throw new Error(`Missing reason for ${file}`);
  for (const code of new Set([...Object.keys(expected), ...Object.keys(actual[file] ?? {})])) {
    const count = actual[file]?.[code] ?? 0;
    if (count !== (expected[code] ?? 0)) {
      console.error(`${file} TS${code}: ${count}, allowed ${expected[code] ?? 0}. Fix new diagnostics; reduce stale allowances.`);
      failed = true;
    }
  }
}
const total = Object.values(actual).flatMap((codes) => Object.values(codes)).reduce((sum, count) => sum + count, 0);
console.log(`Script implicit-any budget: ${total} diagnostics in ${Object.keys(actual).length} files.`);
process.exitCode = failed ? 1 : 0;
