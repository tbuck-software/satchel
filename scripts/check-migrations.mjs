#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, realpathSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const directory = 'crates/satchel-core/src/db/migrations';

export function migrationProblems(current, baseline) {
  const problems = [];
  const seen = new Map();
  const version = (name) => {
    const match = /^(\d+)_.*\.sql$/.exec(name);
    if (!match) throw new Error(`Invalid migration filename: ${name}`);
    return Number(match[1]);
  };
  const highest = Math.max(0, ...Object.keys(baseline).map(version));
  for (const [name, content] of Object.entries(current)) {
    const number = version(name);
    if (seen.has(number)) problems.push(`Migration ${number} is used by ${seen.get(number)} and ${name}`);
    seen.set(number, name);
    if (Object.hasOwn(baseline, name)) {
      if (content !== baseline[name]) problems.push(`Applied migration ${name} was modified`);
    } else if (number <= highest) {
      problems.push(`New migration ${name} must be above ${highest}`);
    }
  }
  for (const name of Object.keys(baseline)) {
    if (!Object.hasOwn(current, name)) problems.push(`Applied migration ${name} was removed or renamed`);
  }
  return problems;
}

export function checkMigrations(base = 'origin/main') {
  const git = (...args) => execFileSync('git', args, { encoding: 'utf8' }).trim();
  const baseline = Object.fromEntries(
    git('ls-tree', '-r', '--name-only', base, directory).split('\n').filter(Boolean)
      .map((path) => [path.slice(directory.length + 1), execFileSync('git', ['show', `${base}:${path}`], { encoding: 'utf8' })]),
  );
  const current = Object.fromEntries(readdirSync(directory).filter((name) => name.endsWith('.sql'))
    .map((name) => [name, readFileSync(`${directory}/${name}`, 'utf8')]));
  const problems = migrationProblems(current, baseline);
  if (problems.length) throw new Error(problems.join('\n'));
  console.log(`Migration versions and existing SQL checked against ${base}.`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  try { checkMigrations(process.argv[2]); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
