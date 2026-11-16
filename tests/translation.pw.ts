import { expect, test } from './mock-contract.fixture';

/*
 * Translate in the reader, against the mock engine (`[DE]` before every
 * text node). The translation must land in the same sandboxed frame behind
 * the same CSP as the original; the WebKit project proves the frame the app
 * ships on.
 */
test.beforeEach(async ({ page }) => {
  await page.goto('/?mockTranslation=ready');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
  await page.getByText('Google AI Studio quota warning', { exact: true }).first().click();
  await expect(page.locator('h1', { hasText: 'Google AI Studio quota warning' })).toBeVisible({ timeout: 15_000 });
});

test('asks once, translates in the protected frame and leads back to the original', async ({ page }) => {
  const frame = page.frameLocator('iframe.satchel-email-document');
  await expect(frame.getByText('Quota resets at midnight UTC').first()).toBeVisible();

  await page.keyboard.press('L');
  const dialog = page.getByRole('dialog', { name: 'Send this message to DeepL?' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Allow and translate' }).click();

  await expect(frame.getByText(/\[DE\] .*Quota resets at midnight UTC/).first()).toBeVisible();
  await expect(page.locator('.translation-note')).toContainText('Translated from English');
  const policy = await page.locator('iframe.satchel-email-document').evaluate((element) => {
    const doc = (element as HTMLIFrameElement).contentDocument;
    return {
      csp: doc?.head.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute('content') ?? '',
      sandbox: element.getAttribute('sandbox') ?? '',
    };
  });
  expect(policy.csp).toContain("default-src 'none'");
  expect(policy.sandbox).not.toContain('allow-scripts');

  await page.locator('.translation-note').getByRole('button', { name: 'Show original' }).click();
  await expect(frame.getByText(/\[DE\]/)).toHaveCount(0);
  await expect(frame.getByText('Quota resets at midnight UTC').first()).toBeVisible();

  // Allowed now: the second time goes without a question.
  await page.keyboard.press('L');
  await expect(frame.getByText(/\[DE\] .*Quota resets at midnight UTC/).first()).toBeVisible();
  await expect(dialog).toHaveCount(0);
});
