import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

test('renders a PDF without a native PDF viewer under the production CSP', async ({ page }) => {
  const csp = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).app.security.csp;
  await page.route('**/index.html', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()).replace('<head>',
      `<head><meta http-equiv="Content-Security-Policy" content="${csp}">`) });
  });
  const violations: string[] = [];
  page.on('console', message => {
    if (/content security policy|refused to/i.test(message.text())) violations.push(message.text());
  });
  await page.goto('/index.html');
  await mountPreview(page);
  const dialog = page.getByRole('dialog', { name: 'Attachment preview' });
  await expect(dialog).toBeVisible();
  const canvas = dialog.locator('canvas');
  await expect(canvas).toBeVisible({ timeout: 12_000 });
  await expect.poll(() => canvas.evaluate((element: HTMLCanvasElement) => {
    const context = element.getContext('2d')!;
    const pixels = context.getImageData(0, 0, element.width, element.height).data;
    let ink = 0;
    for (let i = 0; i < pixels.length; i += 4) {
      if (pixels[i + 3] === 255 && pixels[i]! < 200) ink++;
    }
    return ink;
  })).toBeGreaterThan(1_000);
  await expect(dialog).not.toContainText('This PDF could not be displayed.');
  await expect(dialog.getByText('Page 1 of 2', { exact: true })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Previous page' })).toBeDisabled();
  await dialog.getByRole('button', { name: 'Next page' }).click();
  await expect(dialog.getByText('Page 2 of 2', { exact: true })).toBeVisible();
  await expect(canvas).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Next page' })).toBeDisabled();
  await expect.poll(() => canvas.evaluate((element: HTMLCanvasElement) => {
    const context = element.getContext('2d')!;
    const pixel = context.getImageData(element.width / 2, element.height * 0.75, 1, 1).data;
    return pixel[0]! > pixel[2]!;
  })).toBe(true);
  await page.setViewportSize({ width: 390, height: 844 });
  await expect.poll(() => canvas.evaluate(element => element.getBoundingClientRect().width)).toBeLessThan(390);
  expect(violations).toEqual([]);
});

async function mountPreview(page: Page, card = false) {
  await page.evaluate(async ({ messages, card }) => {
    const vuePath = '/node_modules/.vite/deps/vue.js';
    const previewPath = card ? '/src/components/mail/AttachmentCard.vue' : '/src/components/mail/AttachmentPreview.vue';
    const piniaPath = '/node_modules/.vite/deps/pinia.js';
    const i18nPath = '/src/i18n/index.ts';
    const [{ createApp }, { default: Preview }, { createPinia }, { i18n, i18nReady }] = await Promise.all([
      import(vuePath), import(previewPath), import(piniaPath), import(i18nPath),
    ]);
    await i18nReady;
    // This standalone mount bypasses the lazy parent that normally loads its catalogue.
    i18n.global.mergeLocaleMessage('en', messages);
    const host = document.createElement('div');
    host.dataset.attachmentModalHost = '';
    // Reader ancestors may establish a containing block and clip their children.
    if (card) host.style.cssText = 'position: fixed; top: 200px; left: 200px; width: 216px; height: 90px; transform: translateZ(0); overflow: hidden; z-index: 100;';
    document.body.append(host);
    const props = card
      ? { attachment: { id: 'a-3', filename: 'oauth-config.pdf', contentType: 'application/pdf', size: 184_000 } }
      : { attachmentId: 'a-3' };
    createApp(Preview, props).use(createPinia()).use(i18n).mount(host);
  }, { messages: JSON.parse(readFileSync('src/i18n/en.json', 'utf8')), card });
}

test('shows the external-open fallback for an invalid PDF', async ({ page }) => {
  await page.route('**/src/services/backend/mockPdf.ts', route => route.fulfill({
    contentType: 'application/javascript',
    body: "export const MOCK_PDF_BASE64 = 'aW52YWxpZA==';",
  }));
  await page.goto('/');
  await mountPreview(page);
  const dialog = page.getByRole('dialog', { name: 'Attachment preview' });
  await expect(dialog).toContainText('This PDF could not be displayed.');
  await expect(dialog.locator('.state').getByRole('button', { name: 'Open with default app' })).toBeVisible();
  await expect(dialog.locator('canvas')).toHaveCount(0);
});

test('clicking a PDF card opens a modal above the reader and closes back to the card', async ({ page }) => {
  await page.goto('/');
  await mountPreview(page, true);
  const card = page.locator('[data-attachment-modal-host] .attachment-card__open');
  await card.click();
  const dialog = page.getByRole('dialog', { name: 'Attachment preview' });
  await expect(dialog).toBeVisible();
  const backdrop = page.locator('.shell-backdrop').filter({ has: dialog });
  await expect.poll(() => backdrop.evaluate(element => {
    const rect = element.getBoundingClientRect();
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  })).toEqual({ x: 0, y: 0, width: 1440, height: 900 });
  await expect(dialog.locator('canvas')).toBeVisible({ timeout: 12_000 });
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(card).toBeFocused();
});
