/**
 * The full catalogues, merged from `src/i18n/parts/*.json`, and the helpers
 * that cut subsets from them. Shared by `i18n-merge.mjs` (which writes the
 * generated files) and the Vite plugin `vite-i18n-chunks.mjs` (which serves
 * lazy chunks). Kept free of a shebang: Vite bundles its config's imports.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const partsDir = path.join(root, 'src', 'i18n', 'parts');

function isPlainObject(value) {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function merge(target, source, trail, conflicts) {
  for (const [key, value] of Object.entries(source)) {
    const next = [...trail, key];
    if (isPlainObject(value)) {
      if (!isPlainObject(target[key])) target[key] = {};
      merge(target[key], value, next, conflicts);
      continue;
    }
    if (key in target && target[key] !== value) conflicts.push(next.join('.'));
    target[key] = value;
  }
}

export function sortDeep(value) {
  if (!isPlainObject(value)) return value;
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, sortDeep(value[key])]));
}

export function keyPaths(value, trail = []) {
  if (!isPlainObject(value)) return [trail.join('.')];
  return Object.entries(value).flatMap(([key, child]) => keyPaths(child, [...trail, key]));
}

export function pick(bundle, keys) {
  const result = {};
  for (const key of keys) {
    const segments = key.split('.');
    let source = bundle;
    for (const segment of segments) {
      if (!isPlainObject(source) || !(segment in source)) {
        source = undefined;
        break;
      }
      source = source[segment];
    }
    if (source === undefined) continue;
    let target = result;
    for (const segment of segments.slice(0, -1)) {
      if (!isPlainObject(target[segment])) target[segment] = {};
      target = target[segment];
    }
    target[segments.at(-1)] = source;
  }
  return result;
}

export function buildBundles() {
  const bundles = { en: {}, de: {} };
  const conflicts = [];
  for (const file of readdirSync(partsDir).filter((name) => name.endsWith('.json')).sort()) {
    const part = JSON.parse(readFileSync(path.join(partsDir, file), 'utf8'));
    for (const locale of ['en', 'de']) {
      if (!part[locale]) throw new Error(`${file}: missing "${locale}"`);
      merge(bundles[locale], part[locale], [file], conflicts);
    }
  }
  return { bundles, conflicts };
}

