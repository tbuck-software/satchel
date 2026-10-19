#!/usr/bin/env node
/**
 * Re-vendor the web fonts into public/fonts + src/styles/fonts.css.
 *
 * The app used to pull its fonts from fonts.googleapis.com at
 * runtime. For a local-first mail client that was wrong three ways: it
 * contacted Google on every launch, it failed offline, and it forced the
 * production CSP to allow external stylesheet and font hosts.
 *
 * Run this only when the font list or weights change.
 */
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
// Figtree as one variable face per subset rather than four static cuts: the
// scale has a 650 step, and a static family answers it by snapping to 600 or
// faking the difference.
const FONT_CSS_URL =
  'https://fonts.googleapis.com/css2?family=Figtree:wght@400..700&family=Fira+Code:wght@400;500&display=swap';
// Google serves woff2 only to browsers that advertise support.
const UA =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36';

const outDir = join(root, 'public', 'fonts');
mkdirSync(outDir, { recursive: true });

// License files travel with the redistributed font binaries.
const FONT_LICENSES = ['figtree', 'firacode'];

/** @param {string | URL} url @param {RequestInit} [init] */
async function fetchText(url, init) {
  try {
    return await (await fetch(url, init)).text();
  } catch (error) {
    console.error(`Font vendoring failed: could not fetch ${url}\n${error.message}`);
    console.error('The existing public/fonts stay untouched; retry when the network is reachable.');
    process.exit(1);
  }
}

/** @param {string | URL} url */
async function fetchBuffer(url) {
  try {
    return Buffer.from(await (await fetch(url)).arrayBuffer());
  } catch (error) {
    console.error(`Font vendoring failed: could not download ${url}\n${error.message}`);
    process.exit(1);
  }
}

for (const family of FONT_LICENSES) {
  const license = await fetchText(`https://raw.githubusercontent.com/google/fonts/main/ofl/${family}/OFL.txt`);
  writeFileSync(join(outDir, `OFL-${family}.txt`), license.replace(/[ \t]+$/gm, ''));
}

let css = await fetchText(FONT_CSS_URL, { headers: { 'User-Agent': UA } });
const urls = [...new Set(css.match(/https:\/\/fonts\.gstatic\.com\/[^)]*\.woff2/g) ?? [])];

for (const url of urls) {
  const name = `${createHash('sha1').update(url).digest('hex').slice(0, 10)}.woff2`;
  const bytes = await fetchBuffer(url);
  writeFileSync(join(outDir, name), bytes);
  css = css.replaceAll(url, `/fonts/${name}`);
}

/*
 * `@font-face` descriptors take no custom properties: `var()` there voids the
 * whole descriptor and every bold run gets synthesised. So the weights stay
 * numbers, checked against the scale in tokens.css instead.
 */
const tokensCss = readFileSync(join(root, 'src', 'styles', 'tokens.css'), 'utf8');
const WEIGHT_SCALE = new Set([...tokensCss.matchAll(/--weight-[\w-]+:\s*(\d+);/g)].map((match) => match[1]));

for (const [, weights] of css.matchAll(/font-weight: ([\d ]+);/g)) {
  const offScale = String(weights).trim().split(/\s+/).filter((weight) => !WEIGHT_SCALE.has(weight));
  if (offScale.length > 0) {
    console.error(`Font vendoring failed: weight ${offScale.join(', ')} is not on the scale in tokens.css`);
    process.exit(1);
  }
}

const header = `/*
 * Self-hosted Figtree and Fira Code (SIL Open Font License 1.1).
 *
 * Vendored rather than loaded from fonts.googleapis.com because that
 * contradicted three of this app's own properties: it contacted Google on
 * every launch (leaking IP and usage timing from a mail client), the fonts
 * silently failed offline in an app that advertises offline use, and the
 * production CSP had to be widened to permit it.
 *
 * Regenerate with scripts/vendor-fonts.mjs.
 */

`;
writeFileSync(join(root, 'src', 'styles', 'fonts.css'), header + css);
console.log(`Vendored ${urls.length} font files into public/fonts`);
