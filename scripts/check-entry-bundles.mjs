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
 * startup, add 1.1 KiB (100.5 KiB, initial 233.8 KiB).
 * Main 102 / 236 KiB since 2026-11-01: main measured 100.9 / 234.8 KiB; the
 * composer's conflict close and the command-failure log line add 0.2 / 0.4 KiB
 * (101.1 / 235.2 KiB). Loading the log line lazily saved under 0.2 KiB.
 * Main initial 237 KiB since 2026-11-04: the phone's tab bar and large title
 * measured 235.9 KiB (the tab bar and mailbox sheet already load lazily); the
 * phone search button, Undo restoring the unread mark and the bucket summary
 * renewed on an account change add 0.5 KiB (101.8 / 236.4 KiB).
 * Initial 238 KiB since 2026-11-04: main measured 101.5 / 235.9 KiB; the
 * certificate notices (banner, details, exception) add 1.26 KiB to the German
 * catalogue and nothing to the entry, because their code loads with the
 * connection banner (101.5 / 237.3 KiB).
 * Viewer initial 125 KiB since 2026-11-04: with main's certificate notices
 * and the phone reader's strings together, the German viewer catalogue path
 * measured 124.05 KiB (56 B over); the viewer's code did not grow.
 * Main initial 239 KiB since 2026-11-05: main measured 101.9 / 237.9 KiB; the
 * System appearance has to live in the settings store, which loads at
 * startup to paint the first frame in the right look, and adds 0.1 KiB
 * (238.03 KiB, 29 B over). Its selector and copy load with the settings.
 * Main initial 240 KiB and viewer initial 128 KiB since 2026-11-05: the
 * dependency refresh measured main 238.9 / viewer 124.6 KiB before and
 * 239.6 / 127.3 KiB after. Vue 3.5.44 adds 0.6 KiB to vendor-vue; marked
 * 18.1.2 (CommonMark fixes, backtracking fixes) adds 2.0 KiB to
 * vendor-markdown, which only the viewer loads at startup. No app code grew.
 * Main 103 / 242 KiB since 2026-11-06: main measured 102.0 / 239.7 KiB; the
 * local spam filter adds 0.7 KiB to the entry (Not spam in the thread store,
 * thread actions, keyboard and palette, and the list row's mark, all of
 * which load at startup) and 1.0 KiB of filter copy to the catalogues
 * (102.7 / 241.4 KiB). Its settings page loads with Settings. The Outlook
 * calendar adds 21 B to the entry (the invites store's Outlook sync call,
 * made at startup like the Google one) and 492 B of Outlook copy to the
 * German catalogue, which loads whole before the first frame. The
 * calendar's own code loads with the calendar view (together 102.7 /
 * 241.8 KiB). */
const budgets = {
  'index.html': { entryGzip: 103 * 1024, initialGzip: 242 * 1024, locales: ['src/i18n/en.json', 'src/i18n/de.json'] },
  'tray.html': { entryGzip: 5 * 1024, initialGzip: 75 * 1024, locales: ['src/i18n/tray.en.json', 'src/i18n/tray.de.json'] },
  'viewer.html': { entryGzip: 5 * 1024, initialGzip: 128 * 1024, locales: ['src/i18n/viewer.en.json', 'src/i18n/viewer.de.json'] },
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
