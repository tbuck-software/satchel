import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { keyIndex, mentionsInSource, messageKeys, repoRoot } from '../../scripts/i18n-graph.mjs';
import { SUBSETS } from '../../scripts/i18n-merge.mjs';
import {
  FIRST_PAINT,
  IDLE_PREFETCH,
  OTHER_ENTRIES,
  computeSplit,
  moduleKeys,
  unanchoredKeyCalls,
  unanchoredReason,
} from '../../scripts/i18n-split.mjs';
import en from './en.json';
import startupDe from './startup.de.json';
import startupEn from './startup.en.json';

function keyPaths(value: unknown, trail: string[] = []): string[] {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return [trail.join('.')];
  return Object.entries(value as Record<string, unknown>)
    .flatMap(([key, child]) => keyPaths(child, [...trail, key]));
}

const allKeys = keyPaths(en);
const index = keyIndex(allKeys);
const split = computeSplit(allKeys);
const startup = new Set(split.startupKeys);
const rel = (file: string) => path.relative(repoRoot, file);

/*
 * The main window loads `startup.<locale>.json` before its first frame and
 * every other key as a chunk with the lazy code that needs it (ADR 0017). A
 * key in the wrong place shows up raw on screen, so these re-derive the
 * placement from the graph instead of trusting the generated files. They share
 * the split's own graph reading; the independent check is the raw-key crawl
 * over a mock production build (`tests/i18n-chunks.pw.ts`).
 */
describe('startup catalogue split', () => {
  /*
   * Two PRs that each pass can leave this stale on main together: one adds a
   * key to startup code, the other moves code. The fix is always the same.
   */
  it('ships the generated startup catalogue for the current code', () => {
    for (const [locale, catalogue] of [['en', startupEn], ['de', startupDe]] as const) {
      const shipped = new Set(keyPaths(catalogue));
      const missing = split.startupKeys.filter((key) => !shipped.has(key));
      const extra = [...shipped].filter((key) => !startup.has(key));
      if (missing.length || extra.length) {
        const list = (keys: string[]) => keys.slice(0, 8).join(', ') + (keys.length > 8 ? `, … (${keys.length})` : '');
        throw new Error(
          `src/i18n/startup.${locale}.json is stale; run \`pnpm i18n:build\`.`
            + (missing.length ? `\n  missing: ${list(missing)}` : '')
            + (extra.length ? `\n  extra: ${list(extra)}` : ''),
        );
      }
    }
  });

  it('carries every key code in the startup graph uses', () => {
    const missing = [...split.startupModules].flatMap((file) =>
      [...moduleKeys(file, index)].filter((key) => !startup.has(key)).map((key) => `${rel(file)}: ${key}`));
    expect(missing).toEqual([]);
  });

  it('gives every key lazy code uses a home loaded before that code runs', () => {
    const missing: string[] = [];
    for (const [boundary, chunk] of split.chunks) {
      const own = new Set(chunk.keys);
      for (const file of split.closures.get(boundary) ?? []) {
        for (const key of moduleKeys(file, index)) {
          if (!startup.has(key) && !own.has(key)) missing.push(`${rel(boundary)} > ${rel(file)}: ${key}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it('ships the strings of components that mount on every start with startup', () => {
    const byFile = new Map([...split.chunks.values()].map((chunk) => [rel(chunk.file), chunk]));
    for (const file of FIRST_PAINT) expect(byFile.get(file)?.keys, file).toEqual([]);
    for (const file of IDLE_PREFETCH) expect(byFile.get(file)?.keys.length, file).toBeGreaterThan(0);
  });

  it('places every key some window can use', () => {
    const lazy = new Set([...split.chunks.values()].flatMap((chunk) => chunk.keys));
    const referenced = messageKeys([...split.modules]);
    const homeless = [...referenced].filter((key) => index.has(key) && !startup.has(key) && !lazy.has(key));
    expect(homeless).toEqual([]);
  });

  it('keeps every key Rust sends in startup', () => {
    const rust = ['src-tauri/src/commands/error.rs', 'crates/satchel-core/src/sync/remote_mutations.rs']
      .map((file) => readFileSync(path.join(repoRoot, file), 'utf8')).join('\n');
    const sent = new Set(rust.match(/backend\.errors\.[A-Za-z0-9_]+/g) ?? []);
    expect(sent.size).toBeGreaterThan(0);
    expect([...sent].filter((key) => index.has(key) && !startup.has(key))).toEqual([]);
    expect(allKeys.filter((key) => key.startsWith('backend.') && !startup.has(key))).toEqual([]);
  });

  it('keeps startup well under half of the catalogue', () => {
    expect(
      split.startupKeys.length,
      'Startup holds most keys again: some import pulls lazy views into the static graph of main.ts. '
        + 'Find it with `node scripts/i18n-graph.mjs src/main.ts`.',
    ).toBeLessThan(allKeys.length / 2);
  });

  it('rejects t() calls whose key text the analysis cannot see', () => {
    expect(unanchoredKeyCalls(split.modules)).toEqual([]);
  });

  it.each([
    ["'mail.reply'", null],
    ['`thread.state.${state}`', null],
    ['labelKey', null],
    ['SECTION_KEYS[section]', null],
    ["wide ? 'a.b' : 'a.c'", null],
    ['`${prefix}.title`', 'template without a literal key head'],
    ["prefix + '.title'", 'key built with + (write a template with a literal head instead)'],
    ["'outbox.status.' + status", 'key built with + (write a template with a literal head instead)'],
    ['ok ? `${p}.a` : "x.y"', 'template without a literal key head'],
  ])('judges the key argument %s', (argument, reason) => {
    expect(unanchoredReason(argument)).toBe(reason);
  });

  it('knows the same other windows as the subsets', () => {
    expect([...OTHER_ENTRIES].sort()).toEqual(Object.values(SUBSETS).flatMap((spec) => spec.entries).sort());
  });

  it('serves dev:mock the startup keys of the mock transport', () => {
    const mock = computeSplit(allKeys, { startupRoots: ['src/services/backend/mock.ts'] });
    expect(mock.startupKeys).toEqual(expect.arrayContaining(split.startupKeys));
    expect(mock.startupKeys).toContain('compose.reply.intro');
  });
});

describe('key mentions', () => {
  const fixture = keyIndex([
    'folders.inbox',
    'outbox.status.failed',
    'outbox.status.sending',
    'settings.notifications.scopeAll',
    'settings.notifications.scopeInbox',
    'settings.notifications.title',
    'mail.attachment.one',
    'mail.attachment.many',
    'ok',
  ]);

  it('counts a key kept in a table', () => {
    expect([...mentionsInSource(`const s = { labelKey: 'folders.inbox' };`, fixture)]).toEqual(['folders.inbox']);
  });

  it('counts every key under a concatenated or interpolated head', () => {
    expect([...mentionsInSource(`t('outbox.status.' + status)`, fixture)].sort())
      .toEqual(['outbox.status.failed', 'outbox.status.sending']);
    expect([...mentionsInSource('t(`outbox.status.${status}`)', fixture)].sort())
      .toEqual(['outbox.status.failed', 'outbox.status.sending']);
    expect([...mentionsInSource('t(`settings.notifications.scope${scope}`)', fixture)].sort())
      .toEqual(['settings.notifications.scopeAll', 'settings.notifications.scopeInbox']);
  });

  it('counts the leaves of a named subtree and top-level keys', () => {
    expect([...mentionsInSource(`te('mail.attachment')`, fixture)].sort())
      .toEqual(['mail.attachment.many', 'mail.attachment.one']);
    expect([...mentionsInSource(`t('ok')`, fixture)]).toEqual(['ok']);
  });

  it('ignores namespaces and strings that are not keys', () => {
    expect([...mentionsInSource(`router.push({ name: 'settings' }); fetch('mail.example.com');`, fixture)]).toEqual([]);
  });
});
