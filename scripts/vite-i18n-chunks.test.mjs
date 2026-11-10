import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { repoRoot } from './i18n-graph.mjs';
import { i18nChunks } from './vite-i18n-chunks.mjs';

/*
 * The plugin is the only thing standing between a lazy view and a raw key in
 * the built app: it must wrap the view's import so its chunk loads first, and
 * it must answer the chunk path with exactly that chunk's keys.
 */

/** @param {import('vite').Plugin} plugin @param {string} hook */
const hook = (plugin, hook) => {
  const value = /** @type {Record<string, unknown>} */ (/** @type {unknown} */ (plugin))[hook];
  return /** @type {(this: unknown, ...args: unknown[]) => any} */ (value);
};

const context = { error: (/** @type {string} */ message) => { throw new Error(message); }, warn() {} };
const [serve, wrap] = i18nChunks();
const router = path.join(repoRoot, 'src', 'router', 'index.ts');

describe('i18n chunk plugin', () => {
  it('waits for the settings chunk before importing the settings view', () => {
    const source = readFileSync(router, 'utf8');
    const result = hook(wrap, 'transform').call(context, source, router);
    expect(result.code).toContain(`__satchelWithMessages("views.SettingsView", () => import('@/views/SettingsView.vue'))`);
    expect(result.code.startsWith('import { withMessages as __satchelWithMessages } from "/src/i18n/chunks/with-messages.js";')).toBe(true);
    // Line numbers stay where they were.
    expect(result.code.split('\n').length).toBe(source.split('\n').length);
  });

  it('wraps the imports of a .vue script block request', () => {
    const file = path.join(repoRoot, 'src', 'views', 'SettingsView.vue');
    const result = hook(wrap, 'transform').call(context, readFileSync(file, 'utf8'), `${file}?vue&type=script&setup=true&lang.ts`);
    expect(result.code).toContain('__satchelWithMessages("components.settings.SpamFilterSettings"');
  });

  it('leaves imports alone whose code needs nothing beyond startup', () => {
    const file = path.join(repoRoot, 'src', 'services', 'fileDialog.ts');
    expect(hook(wrap, 'transform').call(context, readFileSync(file, 'utf8'), file)).toBeNull();
  });

  it('leaves the viewer alone, whose subset is complete', () => {
    const file = path.join(repoRoot, 'src', 'viewer', 'MessageFileViewer.vue');
    expect(hook(wrap, 'transform').call(context, readFileSync(file, 'utf8'), file)).toBeNull();
  });

  it('does not touch test files or style requests', () => {
    const code = "const view = () => import('@/views/SettingsView.vue');";
    expect(hook(wrap, 'transform').call(context, code, path.join(repoRoot, 'src', 'x.test.ts'))).toBeNull();
    expect(hook(wrap, 'transform').call(context, code, `${router}?vue&type=style`)).toBeNull();
  });

  it('answers a chunk path with that chunk in the requested locale', () => {
    const id = hook(serve, 'resolveId').call(context, '/src/i18n/chunks/views.SettingsView.de.json');
    expect(id).toBe(path.join(repoRoot, 'src', 'i18n', 'chunks', 'views.SettingsView.de.json'));
    const messages = JSON.parse(hook(serve, 'load').call(context, id));
    expect(messages.settings).toBeTypeOf('object');
    expect(JSON.stringify(messages)).not.toContain('"backend"');
  });

  it('lists each chunk with a loader per locale in the table', () => {
    const id = hook(serve, 'resolveId').call(context, '/src/i18n/chunks/table.js');
    const table = hook(serve, 'load').call(context, id);
    expect(table).toContain(
      '"views.SettingsView": { en: () => import("/src/i18n/chunks/views.SettingsView.en.json"), '
        + 'de: () => import("/src/i18n/chunks/views.SettingsView.de.json") },',
    );
    const helper = hook(serve, 'load').call(context, hook(serve, 'resolveId').call(context, '/src/i18n/chunks/with-messages.js'));
    expect(helper).toContain("from '@/i18n/messageChunks'");
  });

  // Vite writes module ids with `/` on every platform. On Windows these are
  // the ids the build hands the plugin; elsewhere the test is the same as above.
  it('takes ids in Vite\'s form, with `/` on every platform', () => {
    const viteId = (/** @type {string} */ file) => file.split(path.sep).join('/');
    const result = hook(wrap, 'transform').call(context, readFileSync(router, 'utf8'), viteId(router));
    expect(result.code).toContain('__satchelWithMessages("views.SettingsView"');
    const chunk = viteId(path.join(repoRoot, 'src', 'i18n', 'chunks', 'views.SettingsView.en.json'));
    expect(JSON.parse(hook(serve, 'load').call(context, chunk)).settings).toBeTypeOf('object');
    expect(hook(serve, 'load').call(context, viteId(path.join(repoRoot, 'src', 'i18n', 'chunks', 'table.js'))))
      .toContain('"views.SettingsView"');
  });

  it('refuses a chunk it does not know', () => {
    const id = path.join(repoRoot, 'src', 'i18n', 'chunks', 'views.Nowhere.en.json');
    expect(() => hook(serve, 'load').call(context, id)).toThrow('unknown message chunk');
  });
});
