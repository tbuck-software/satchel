import { readFileSync } from 'node:fs';
import type { Page } from '@playwright/test';
import { expect, test } from './mock-contract.fixture';
import { IDLE_PREFETCH, chunkId } from '../scripts/i18n-split.mjs';

/*
 * The main window loads only the startup catalogue before its first frame;
 * lazy views bring their strings as chunks (ADR 0017). A key that missed its
 * chunk renders as itself, so open the lazy surfaces in both languages and
 * look for catalogue keys on screen and for the app's missing-key warning.
 *
 * This is the check that does not share the split's own reasoning: it runs in
 * the smoke suite against the dev server and, in CI, against a mock production
 * build (tests/playwright.i18n.config.ts), where chunks are precompiled and
 * hashed exactly as they ship.
 */

function keyPaths(value: unknown, trail: string[] = []): string[] {
  if (typeof value !== 'object' || value === null) return [trail.join('.')];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) => keyPaths(child, [...trail, key]));
}

const en = JSON.parse(readFileSync(new URL('../src/i18n/en.json', import.meta.url), 'utf8'));
const catalogue = new Set(keyPaths(en));

const VIEWS = ['/inbox', '/later', '/calendar', '/tasks', '/contacts', '/outbox', '/drafts', '/cleanup', '/new-senders', '/search', '/compose'];

/** A message chunk request: `/src/i18n/chunks/<id>.<locale>.json` in dev, `<id>.<locale>-<hash>.js` built. */
const CHUNK_URL = /\/([A-Za-z]+\.[\w.]+)\.(en|de)(?:\.json|-[\w-]+\.js)(?:\?|$)/;

function watch(page: Page) {
  const missing: string[] = [];
  const chunks = new Set<string>();
  page.on('console', (message) => {
    if (message.text().startsWith('[i18n] missing key')) missing.push(message.text());
  });
  page.on('request', (request) => {
    const match = CHUNK_URL.exec(new URL(request.url()).pathname);
    if (match) chunks.add(match[1]!);
  });
  return { missing, chunks };
}

async function rawKeys(page: Page, where: string): Promise<string[]> {
  await page.waitForLoadState('networkidle');
  const text = await page.evaluate(() => {
    const parts = [document.body.innerText];
    for (const element of Array.from(document.querySelectorAll('[aria-label], [title], [placeholder]'))) {
      for (const name of ['aria-label', 'title', 'placeholder']) parts.push(element.getAttribute(name) ?? '');
    }
    return parts.join('\n');
  });
  return [...new Set(text.match(/[a-z][A-Za-z0-9_-]*(?:\.[A-Za-z0-9_-]+)+/g) ?? [])]
    .filter((token) => catalogue.has(token))
    .map((key) => `${where}: ${key}`);
}

async function startIn(page: Page, locale: 'en' | 'de') {
  await page.addInitScript((value) => {
    window.localStorage.setItem('satchel.locale', value);
    // A returning user: a fresh profile would show onboarding first.
    window.localStorage.setItem('satchel.onboardingComplete', 'true');
  }, locale);
}

for (const locale of ['en', 'de'] as const) {
  test.describe(`lazy surfaces in ${locale}`, () => {
    test.beforeEach(async ({ page }) => startIn(page, locale));

    test('settings sections show no raw keys', async ({ page }) => {
      test.setTimeout(240_000);
      const { missing } = watch(page);
      await page.goto('/settings');
      const dialog = page.getByRole('dialog').first();
      await expect(dialog).toBeVisible();
      // Every section the sidebar offers here; the ones a platform hides are
      // not reachable anyway.
      const sections = dialog.getByRole('navigation').first().getByRole('listitem').getByRole('button');
      // The sidebar is lazy itself; count once it is there.
      await expect(sections.first()).toBeVisible();
      const count = await sections.count();
      expect(count).toBeGreaterThan(10);
      const found: string[] = [];
      for (let index = 0; index < count; index += 1) {
        const section = sections.nth(index);
        const name = (await section.textContent())?.trim() ?? String(index);
        await section.click();
        found.push(...await rawKeys(page, name));
      }
      expect(found).toEqual([]);
      expect(missing).toEqual([]);
    });

    test('views show no raw keys', async ({ page }) => {
      const { missing } = watch(page);
      const found: string[] = [];
      for (const view of VIEWS) {
        await page.goto(view);
        found.push(...await rawKeys(page, view));
      }
      expect(found).toEqual([]);
      expect(missing).toEqual([]);
    });

    test('the reader, shortcut help and palette show no raw keys', async ({ page }) => {
      const { missing } = watch(page);
      await page.goto('/inbox');
      await page.locator('[role="option"]').first().click();
      const found = await rawKeys(page, 'reader');
      await page.locator('body').press('Escape');
      await page.locator('body').press('?');
      await expect(page.getByRole('dialog').first()).toBeVisible();
      found.push(...await rawKeys(page, 'shortcut help'));
      await page.keyboard.press('Escape');
      await page.keyboard.press('ControlOrMeta+k');
      await expect(page.getByRole('dialog').first()).toBeVisible();
      found.push(...await rawKeys(page, 'palette'));
      expect(found).toEqual([]);
      expect(missing).toEqual([]);
    });
  });
}

/*
 * What a cold start loads besides the startup catalogue. Components that
 * mount on every start belong in FIRST_PAINT (scripts/i18n-split.mjs), whose
 * keys ship in startup; the reader and palette are warmed once the Inbox is
 * usable (IDLE_PREFETCH). The rest below are what the mock's data shows on
 * every start: a failed sync and an outbox item that needs attention.
 */
const MOCK_STATE_CHUNKS = ['components.layout.SyncFailures', 'components.ui.ToastStack', 'components.ui.UndoFab'];
const ALLOWED_AT_START = new Set([
  ...IDLE_PREFETCH.map((file) => chunkId(new URL(`../${file}`, import.meta.url).pathname)),
  ...MOCK_STATE_CHUNKS,
]);

for (const viewport of [{ name: 'desktop', width: 1440, height: 900 }, { name: 'phone', width: 390, height: 844 }]) {
  test(`a cold start on the ${viewport.name} loads no chunk beyond the idle prefetch`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await startIn(page, 'en');
    const { chunks, missing } = watch(page);
    await page.goto('/inbox');
    await expect(page.locator('[role="option"]').first()).toBeVisible();
    await page.waitForLoadState('networkidle');
    expect([...chunks].filter((id) => !ALLOWED_AT_START.has(id)).sort()).toEqual([]);
    expect(missing).toEqual([]);
  });
}

test('switching to German translates a lazy view that is already open', async ({ page }) => {
  await startIn(page, 'en');
  const { missing } = watch(page);
  await page.goto('/settings?section=general');
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings).toBeVisible();
  await settings.getByRole('group', { name: 'Language' }).getByRole('button', { name: 'Deutsch' }).click();
  await expect(page.getByRole('dialog', { name: 'Einstellungen' })).toBeVisible();
  expect(await rawKeys(page, 'after switch')).toEqual([]);
  expect(missing).toEqual([]);
});
