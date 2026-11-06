import { readFileSync } from 'node:fs';
import type { Page } from '@playwright/test';
import { expect, test } from './mock-contract.fixture';

/*
 * The main window loads only the startup catalogue before its first frame;
 * lazy views bring their strings as chunks (ADR 0017). A key that missed its
 * chunk renders as itself, so look for catalogue keys on screen after opening
 * the lazy views, in both languages.
 */

function keyPaths(value: unknown, trail: string[] = []): string[] {
  if (typeof value !== 'object' || value === null) return [trail.join('.')];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) => keyPaths(child, [...trail, key]));
}

const catalogue = new Set(keyPaths(JSON.parse(readFileSync(new URL('../src/i18n/en.json', import.meta.url), 'utf8'))));

async function rawKeys(page: Page): Promise<string[]> {
  const text = await page.evaluate(() => {
    const parts = [document.body.innerText];
    for (const element of Array.from(document.querySelectorAll('[aria-label], [title], [placeholder]'))) {
      for (const name of ['aria-label', 'title', 'placeholder']) parts.push(element.getAttribute(name) ?? '');
    }
    return parts.join('\n');
  });
  return [...new Set(text.match(/[a-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+/g) ?? [])].filter((token) => catalogue.has(token));
}

const SETTINGS_SECTIONS = ['general', 'accounts', 'senders', 'rules', 'vacation', 'calendar', 'storage', 'plugins', 'diagnostics', 'templates'];
const VIEWS = ['/calendar', '/contacts', '/outbox', '/drafts', '/cleanup', '/new-senders', '/search'];

for (const locale of ['en', 'de'] as const) {
  test.describe(`lazy views in ${locale}`, () => {
    test.beforeEach(async ({ page }) => {
      await page.addInitScript((value) => window.localStorage.setItem('satchel.locale', value), locale);
    });

    test('settings sections show no raw keys', async ({ page }) => {
      const found: string[] = [];
      for (const section of SETTINGS_SECTIONS) {
        await page.goto(`/settings?section=${section}`);
        await expect(page.getByRole('dialog').first()).toBeVisible();
        await page.waitForLoadState('networkidle');
        found.push(...(await rawKeys(page)).map((key) => `${section}: ${key}`));
      }
      expect(found).toEqual([]);
    });

    test('views show no raw keys', async ({ page }) => {
      const found: string[] = [];
      for (const view of VIEWS) {
        await page.goto(view);
        await page.waitForLoadState('networkidle');
        found.push(...(await rawKeys(page)).map((key) => `${view}: ${key}`));
      }
      expect(found).toEqual([]);
    });
  });
}

test('switching to German translates a lazy view that is already open', async ({ page }) => {
  await page.goto('/settings?section=general');
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings).toBeVisible();
  await settings.getByRole('group', { name: 'Language' }).getByRole('button', { name: 'Deutsch' }).click();
  await expect(page.getByRole('dialog', { name: 'Einstellungen' })).toBeVisible();
  await page.waitForLoadState('networkidle');
  expect(await rawKeys(page)).toEqual([]);
});
