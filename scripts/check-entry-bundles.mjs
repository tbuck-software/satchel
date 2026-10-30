#!/usr/bin/env node
import { existsSync, readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const manifestPath = path.join(root, 'dist', '.vite', 'manifest.json');

if (!existsSync(manifestPath)) {
  throw new Error('dist/.vite/manifest.json is missing; run `pnpm build` first');
}

/** @type {import("vite").Manifest} */
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
/* Startup includes the chosen catalogue, even though it is a dynamic import.
 * The inactive locale is excluded; measure the larger of the two active paths.
 * Measured 2026-10-13 after the review integration: main initial 227.6 KiB,
 * viewer 120.1 KiB, tray 70.2 KiB gzip (was 238.1 / 118.1 / 73.0 before).
 * Main entry 101 KiB since 2026-10-30: main measured 99.4 KiB, and the IMAP
 * generation checks in the messages and drafts stores, which load at
 * startup, add 1.1 KiB (100.5 KiB, initial 233.8 KiB). */
const budgets = {
  'index.html': { entryGzip: 101 * 1024, initialGzip: 235 * 1024, locales: ['src/i18n/en.json', 'src/i18n/de.json'] },
  'tray.html': { entryGzip: 5 * 1024, initialGzip: 75 * 1024, locales: ['src/i18n/tray.en.json', 'src/i18n/tray.de.json'] },
  'viewer.html': { entryGzip: 5 * 1024, initialGzip: 124 * 1024, locales: ['src/i18n/viewer.en.json', 'src/i18n/viewer.de.json'] },
};

/**
 * @param {string} entry
 */
function staticGraph(entry) {
  /** @type {Set<string>} */
  const seen = new Set();
  /** @param {string} key */
  const visit = (key) => {
    if (seen.has(key)) return;
    seen.add(key);
    for (const imported of manifest[key]?.imports ?? []) visit(imported);
  };
  visit(entry);
  return seen;
}

/**
 * @param {string} file
 */
function gzipBytes(file) {
  return gzipSync(readFileSync(path.join(root, 'dist', file))).byteLength;
}

let failed = false;
for (const [entry, budget] of Object.entries(budgets)) {
  const chunk = manifest[entry];
  if (!chunk?.isEntry || !chunk.file) throw new Error(`missing Vite entry: ${entry}`);
  const graph = staticGraph(entry);
  for (const locale of budget.locales) {
    if (!manifest[locale]?.file) throw new Error(`missing locale chunk: ${locale}`);
    if (graph.has(locale)) throw new Error(`locale must load on demand: ${locale}`);
  }
  /** @param {Set<string>} keys */
  const graphBytes = keys => [...keys]
    .map(key => manifest[key]?.file)
    .filter(file => file?.endsWith('.js'))
    .reduce((sum, file) => sum + gzipBytes(file), 0);
  const initialGzip = Math.max(...budget.locales.map(locale => graphBytes(new Set([...graph, ...staticGraph(locale)]))));
  const entryGzip = gzipBytes(chunk.file);
  process.stdout.write(
    `${entry}: entry ${(entryGzip / 1024).toFixed(1)} KiB gzip, `
      + `initial JS ${(initialGzip / 1024).toFixed(1)} KiB gzip\n`,
  );
  if (entryGzip > budget.entryGzip || initialGzip > budget.initialGzip) {
    failed = true;
    process.stderr.write(
      `${entry} exceeds its budget: entry <= ${(budget.entryGzip / 1024).toFixed(0)} KiB, `
        + `initial JS <= ${(budget.initialGzip / 1024).toFixed(0)} KiB gzip\n`,
    );
  }
}

for (const name of ['viewer.en.json', 'viewer.de.json', 'tray.en.json', 'tray.de.json']) {
  const bytes = readFileSync(path.join(root, 'src', 'i18n', name)).byteLength;
  process.stdout.write(`${name}: ${(bytes / 1024).toFixed(1)} KiB\n`);
  if (bytes > 16 * 1024) {
    failed = true;
    process.stderr.write(`${name} exceeds the 16 KiB catalogue budget\n`);
  }
}

if (failed) process.exitCode = 1;
