import { spawnSync } from 'node:child_process';

// Include every platform and app feature. Dev edges stay out of the graph,
// so tests can still enable test-support, including under --release.
const graph = spawnSync('cargo', [
  'tree', '-p', 'satchel', '--locked', '--target', 'all', '--all-features',
  '--edges', 'no-dev', '--invert', 'satchel-core', '--prefix', 'none', '--format', '{p}\t{f}',
], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });

if (graph.error || graph.status !== 0) {
  console.error(graph.error?.message ?? 'Could not inspect the Satchel release feature graph.');
  process.exit(graph.status ?? 1);
}

const corePackages = graph.stdout.split(/\r?\n/)
  .map((line) => line.split('\t'))
  .filter(([name]) => name.startsWith('satchel-core '));
if (corePackages.length === 0 || corePackages.some(([, features]) => features === undefined)) {
  console.error('Could not find satchel-core features in the Satchel release graph.');
  process.exit(1);
}
if (corePackages.some(([, features]) => features.split(/[,\s]+/).includes('test-support'))) {
  console.error('satchel-core/test-support must be enabled only by dev dependencies.');
  process.exit(1);
}

console.log('Satchel release dependencies exclude satchel-core/test-support.');
