import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const result = spawnSync(process.execPath, [require.resolve('typescript/bin/tsc'), '-p', 'tsconfig.scripts.json', '--pretty', 'false'], {
  stdio: 'inherit',
});
if (result.error || result.status === null) throw result.error ?? new Error('TypeScript did not exit normally');
process.exitCode = result.status;
