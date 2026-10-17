#!/usr/bin/env node
/**
 * Module graph of an entry, plus the message keys it references.
 *
 * Used by `src/i18n/subsets.test.ts` to prove that the viewer and tray
 * catalogues really cover the components those windows mount — the whole point
 * of the separate entries is that they do not import the client's catalogue,
 * so a missing key would only show up as a raw key on screen.
 */
import { readFileSync, existsSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const CANDIDATE_SUFFIXES = ['', '.ts', '.vue', '/index.ts'];

function resolveImport(spec, fromFile) {
  const base = spec.startsWith('@/')
    ? path.join(repoRoot, 'src', spec.slice(2))
    : spec.startsWith('.')
      ? path.resolve(path.dirname(fromFile), spec)
      : null;
  if (!base) return null;
  for (const suffix of CANDIDATE_SUFFIXES) {
    const candidate = base + suffix;
    if (!/\.(ts|vue)$/.test(candidate)) continue;
    if (existsSync(candidate) && statSync(candidate).isFile()) return candidate;
  }
  return null;
}

const IMPORT_RE = /(?:from\s+|import\s*\(\s*)['"]([^'"]+)['"]/g;

export function moduleGraph(entries) {
  const seen = new Set();
  const queue = entries.map((entry) => path.join(repoRoot, entry));

  while (queue.length) {
    const file = queue.pop();
    if (seen.has(file) || !existsSync(file)) continue;
    seen.add(file);
    const source = readFileSync(file, 'utf8');
    for (const match of source.matchAll(IMPORT_RE)) {
      const resolved = resolveImport(match[1], file);
      if (resolved && !seen.has(resolved)) queue.push(resolved);
    }
  }
  return [...seen].sort();
}

const KEY_RE = /(?:\$t|\btc?)\(\s*(['"])([a-zA-Z][\w.]*)\1/g;

export function messageKeys(files) {
  const keys = new Set();
  for (const file of files) {
    const source = readFileSync(file, 'utf8');
    for (const match of source.matchAll(KEY_RE)) keys.add(match[2]);
  }
  return keys;
}

if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) {
  const files = moduleGraph(process.argv.slice(2));
  process.stdout.write(files.map((file) => path.relative(repoRoot, file)).join('\n'));
  process.stdout.write(`\nTOTAL ${files.length}\n`);
}
