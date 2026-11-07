#!/usr/bin/env node
import { existsSync, readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { FIRST_PAINT, IDLE_PREFETCH, chunkId } from './i18n-split.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const manifestPath = path.join(root, 'dist', '.vite', 'manifest.json');

if (!existsSync(manifestPath)) {
  throw new Error('dist/.vite/manifest.json is missing; run `pnpm build` first');
}

/** @type {import("vite").Manifest} */
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
/* Startup includes the chosen catalogue, even though it is a dynamic import.
 * For the main window that is the startup catalogue; view chunks are not
 * counted, since they load with their views.
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
 * 241.8 KiB). Main 104 KiB entry since then: the mailbox quota marker in
 * the sidebar needs its store and level helper at startup (the account row
 * is painted in the first frame) and adds 0.5 KiB to the entry and 1.0 KiB
 * with its copy (together 103.2 / 242.8 KiB). The settings card and its
 * meter load with the settings. The server rules (ManageSieve) add 1.6 KiB
 * to the German catalogue and nothing to the entry; their panel, rule
 * badges and backend calls load with the Accounts, Rules and Out-of-office
 * sections, while the strings cannot, because the one per-locale catalogue
 * loads at startup (together 103.2 / 244.4 KiB). The sender cleanup adds
 * 0.6 KiB to the entry (its dialog state, key handler and route; the sweep,
 * its store, the dialog and the cleanup view load on first use) and 2.7 KiB
 * of copy to the catalogues (together 103.8 / 247.2 KiB, with the composer
 * tables). Nearly all of this wave's growth is copy; splitting the
 * catalogue so that view strings load with their views is the way back.
 * Main 105 / 210 KiB since 2026-11-06 (ADR 0017): startup loads only
 * src/i18n/startup.<locale>.json, the keys of the static graph of main.ts
 * plus those of the lazy components that mount on every start (FIRST_PAINT);
 * the rest load as chunks with the lazy code. Main measured 104.5 / 209.6 KiB
 * (was 103.8 / 247.2), the German catalogue 21.5 KiB of it. The entry grows
 * 0.7 KiB: the chunk loader (i18n/messageChunks) and the wrapped dynamic
 * imports of startup modules, which name their chunk id; the table of chunk
 * files loads on first use. Viewer 127.6 and tray 72.0 KiB (were 127.4 /
 * 71.8): the locale-loader hook in i18n/runtime and the missing-key warning
 * in i18n/create, which they share. After the
 * idle prefetch the German catalogue measures 31.8 KiB (budget below).
 * Main 211 KiB initial and 33 KiB after prefetch since 2026-11-07: mail
 * translation measured 210.3 / 32.6 KiB on main's 209.6 / 31.8; its twenty
 * backend.errors.translation messages ship at startup like every backend.*
 * key (+0.7 KiB first paint), and the reader's Translate strings ride in the
 * prefetched ThreadView and OmniSearch chunks, while the consent question,
 * the Translation page and the settings search part load on use.
 * Main 106 / 212 KiB since 2026-11-07: with tasks main measured 105.1 /
 * 211.3 KiB and 32.9 KiB after prefetch (was 104.5 / 210.3 / 32.6), the
 * entry carrying the task keys, palette commands, sidebar button and editor
 * state, and the Today card (FIRST_PAINT) its due-task line with the store
 * it reads; the view, editor and their strings load on first use.
 * Main 213 and viewer 129 KiB since 2026-11-07: mail merge measured 105.2 /
 * 212.1 and viewer 128.2 KiB on main's 105.1 / 211.4 and 127.7, from its
 * backend.errors copy in the startup catalogue (German 22.6 → 23.1 KiB) and,
 * in the viewer, the mail document helpers splitting into a chunk of their
 * own now that the merge preview also renders through IsolatedEmailDocument. */
const budgets = {
  'index.html': { entryGzip: 106 * 1024, initialGzip: 213 * 1024, locales: ['src/i18n/startup.en.json', 'src/i18n/startup.de.json'] },
  'tray.html': { entryGzip: 5 * 1024, initialGzip: 75 * 1024, locales: ['src/i18n/tray.en.json', 'src/i18n/tray.de.json'] },
  'viewer.html': { entryGzip: 5 * 1024, initialGzip: 129 * 1024, locales: ['src/i18n/viewer.en.json', 'src/i18n/viewer.de.json'] },
};

/* The startup catalogue plus the reader's and palette's chunks, which load
 * once the Inbox is usable. The first-paint part is inside initialGzip above.
 * Free/busy puts only its four backend.errors.availability* messages into the
 * startup catalogue (every backend.* key ships there) and the rest of its
 * copy into its lazy chunks: German 32.9 → 33.1 KiB after prefetch, initial
 * JS 211.3 → 211.4 KiB, entry unchanged at 105.1 KiB. */
const CATALOGUE_AFTER_PREFETCH_GZIP = 34 * 1024;

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

/*
 * The main window's catalogue as it loads (ADR 0017). Before the first frame:
 * the startup catalogue only. Components that mount on every start ship their
 * keys in it (FIRST_PAINT), so none of them may have a chunk; the raw-key
 * crawl (tests/i18n-chunks.pw.ts) fails when a cold start requests any other
 * chunk. Once the Inbox is usable, bootstrap warms the reader and the palette
 * (IDLE_PREFETCH) and their chunks come along.
 */
{
  /** @param {string} id @param {string} locale */
  const chunkFile = (id, locale) => manifest[`src/i18n/chunks/${id}.${locale}.json`]?.file;
  for (const file of FIRST_PAINT) {
    const id = chunkId(path.join(root, file));
    if (chunkFile(id, 'en')) {
      failed = true;
      process.stderr.write(`${file} mounts on every start but has a message chunk (${id})\n`);
    }
  }
  const catalogue = ['en', 'de'].map((locale) => {
    const startup = gzipBytes(/** @type {string} */ (manifest[`src/i18n/startup.${locale}.json`]?.file));
    const prefetch = IDLE_PREFETCH.reduce((sum, file) => {
      const chunk = chunkFile(chunkId(path.join(root, file)), locale);
      if (!chunk) throw new Error(`missing idle prefetch chunk: ${file} (${locale})`);
      return sum + gzipBytes(chunk);
    }, 0);
    return { locale, startup, prefetch };
  });
  for (const { locale, startup, prefetch } of catalogue) {
    process.stdout.write(
      `index.html catalogue (${locale}): first paint ${(startup / 1024).toFixed(1)} KiB gzip, `
        + `after idle prefetch ${((startup + prefetch) / 1024).toFixed(1)} KiB gzip\n`,
    );
  }
  const largest = Math.max(...catalogue.map(({ startup, prefetch }) => startup + prefetch));
  if (largest > CATALOGUE_AFTER_PREFETCH_GZIP) {
    failed = true;
    process.stderr.write(
      `index.html catalogue after idle prefetch exceeds ${(CATALOGUE_AFTER_PREFETCH_GZIP / 1024).toFixed(0)} KiB gzip\n`,
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
