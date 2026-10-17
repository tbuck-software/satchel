import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';

/**
 * Verifies the production Content-Security-Policy against the real built app.
 *
 * The policy is configured in tauri.conf.json and applied by the webview, so
 * nothing in the normal test suite exercises it — a policy that blocks the
 * app's own assets would only show up when someone runs the desktop build.
 * This loads dist/ with the same policy applied as a meta tag and fails on any
 * violation.
 *
 * Caveat: meta-tag CSP cannot express frame-ancestors or sandbox (we use
 * neither), and Tauri's own ipc: transport is not exercised here.
 */
function productionCsp(): string {
  const config = JSON.parse(
    readFileSync(fileURLToPath(new URL('../src-tauri/tauri.conf.json', import.meta.url)), 'utf8'),
  );
  const csp = config?.app?.security?.csp;
  expect(csp, 'tauri.conf.json must define a CSP').toBeTruthy();
  expect(csp, 'production CSP must not allow runtime code evaluation').not.toContain(
    "'unsafe-eval'",
  );
  return csp;
}

test('the production CSP does not block the built app', async ({ page }) => {
  const violations: string[] = [];
  const failedRequests: string[] = [];

  page.on('console', (message) => {
    const text = message.text();
    if (/content security policy|refused to/i.test(text)) {
      violations.push(text);
    }
  });
  page.on('requestfailed', (request) => {
    failedRequests.push(`${request.url()} — ${request.failure()?.errorText ?? 'unknown'}`);
  });

  // Inject the real policy before anything else in <head>, the way the webview
  // applies it.
  const csp = productionCsp();
  await page.route('**/index.html', async (route) => {
    const response = await route.fetch();
    const body = (await response.text()).replace(
      '<head>',
      `<head><meta http-equiv="Content-Security-Policy" content="${csp.replace(/"/g, '&quot;')}">`,
    );
    await route.fulfill({ response, body, headers: { ...response.headers(), 'content-type': 'text/html' } });
  });

  await page.goto('/index.html');
  await page.waitForLoadState('networkidle');

  expect(
    violations,
    `The production CSP blocks resources the app needs:\n  ${violations.join('\n  ')}`,
  ).toEqual([]);
  expect(
    failedRequests.filter((entry) => /blocked|csp/i.test(entry)),
    `Requests blocked by the CSP:\n  ${failedRequests.join('\n  ')}`,
  ).toEqual([]);
});

test('the app renders with the production CSP applied', async ({ page }) => {
  const csp = productionCsp();
  await page.route('**/index.html', async (route) => {
    const response = await route.fetch();
    const body = (await response.text()).replace(
      '<head>',
      `<head><meta http-equiv="Content-Security-Policy" content="${csp.replace(/"/g, '&quot;')}">`,
    );
    await route.fulfill({ response, body, headers: { ...response.headers(), 'content-type': 'text/html' } });
  });

  await page.goto('/index.html');
  // If script-src blocked the bundle, nothing would mount at all.
  await expect(page.locator('#app')).not.toBeEmpty();
});

for (const entry of ['index', 'viewer', 'tray'] as const) {
  for (const locale of ['en', 'de'] as const) {
    test(`${entry} loads only its active ${locale} catalogue under the production CSP`, async ({ page }) => {
      const manifest = JSON.parse(readFileSync('dist/.vite/manifest.json', 'utf8'));
      const prefix = entry === 'index' ? '' : `${entry}.`;
      const activeChunk = manifest[`src/i18n/${prefix}${locale}.json`].file;
      const inactiveChunk = manifest[`src/i18n/${prefix}${locale === 'en' ? 'de' : 'en'}.json`].file;
      const requests: string[] = [];
      const errors: string[] = [];
      page.on('request', request => requests.push(request.url()));
      page.on('pageerror', error => errors.push(error.message));
      await page.addInitScript(locale => localStorage.setItem('satchel.locale', locale), locale);
      const csp = productionCsp();
      await page.route(`**/${entry}.html`, async route => {
        const response = await route.fetch();
        const body = (await response.text()).replace('<head>', `<head><meta http-equiv="Content-Security-Policy" content="${csp.replace(/"/g, '&quot;')}">`);
        await route.fulfill({ response, body });
      });
      await page.goto(`/${entry}.html`);
      await expect(page.locator('#app')).not.toBeEmpty();
      await expect(page.locator('html')).toHaveAttribute('lang', locale);
      if (entry === 'viewer') {
        await expect(page.getByText(locale === 'en' ? 'No file path was passed.' : 'Kein Dateipfad übergeben.', { exact: true })).toBeVisible();
      } else if (entry === 'tray') {
        await expect(page.getByRole('main', { name: locale === 'en' ? 'Satchel briefing' : 'Satchel Briefing' })).toBeVisible();
      }
      expect(requests.some(url => url.endsWith(activeChunk))).toBe(true);
      expect(requests.some(url => url.endsWith(inactiveChunk))).toBe(false);
      expect(errors).toEqual([]);
    });
  }
}
