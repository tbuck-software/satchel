import { expect, test } from './mock-contract.fixture';

/*
 * The mail body is a frame sandboxed without `allow-scripts`, and WebKit runs
 * no listener inside it. A click into the text used to leave the keyboard in
 * the frame, so every shortcut was dead until the next click outside it —
 * in the shipped WKWebView, never in Chromium.
 */
test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
  await page.getByText('Google AI Studio quota warning', { exact: true }).first().click();
  await expect(page.locator('h1', { hasText: 'Google AI Studio quota warning' })).toBeVisible({ timeout: 15_000 });
});

test('moves through the list with j after a click into the mail body', async ({ page }) => {
  const body = page.frameLocator('iframe.satchel-email-document').getByText('Quota resets at midnight UTC');
  await body.click();

  const keyboardFocus = page.locator('.list-target--keyboard-focused, .thread-row--keyboard-focused');
  await expect(keyboardFocus).toHaveCount(0);

  await page.keyboard.press('j');

  await expect(keyboardFocus).toHaveCount(1);
});

test('keeps a text selection in the mail while the shortcuts work again', async ({ page, browserName }) => {
  // In Chromium a mouse drag inside the scriptless frame never resolves under
  // Playwright, with or without the refocus; WebKit is the engine at issue.
  test.skip(browserName === 'chromium', 'drag inside the sandboxed frame hangs in Playwright Chromium');
  const frame = page.locator('iframe.satchel-email-document');
  await expect(frame).toBeVisible();
  const line = await page.frameLocator('iframe.satchel-email-document')
    .getByText('Requests today: 1,840 of 2,000')
    .boundingBox();
  if (!line) throw new Error('mail line not found');
  await page.mouse.move(line.x + 2, line.y + line.height / 2);
  await page.mouse.down();
  await page.mouse.move(line.x + 120, line.y + line.height / 2, { steps: 8 });
  await page.mouse.up();

  const selected = () => frame.evaluate((element) => (
    (element as HTMLIFrameElement).contentDocument?.getSelection()?.toString() ?? ''
  ));
  await expect.poll(selected).not.toBe('');
  await expect.poll(() => page.evaluate(() => document.activeElement?.tagName)).not.toBe('IFRAME');
  expect(await selected()).not.toBe('');

  await page.keyboard.press('j');
  await expect(page.locator('.list-target--keyboard-focused, .thread-row--keyboard-focused')).toHaveCount(1);
});

for (const undo of ['keyboard', 'toast'] as const) {
  test(`undo restores the visible row, reader and focus through ${undo}`, async ({ page }) => {
    const row = page.locator('[data-list-target-key="thread:th-7"]');
    await page.keyboard.press('e');
    await expect(row).toHaveCount(0);
    await page.keyboard.press('j');
    await page.keyboard.press('k');
    if (undo === 'keyboard') await page.keyboard.press('z');
    else await page.locator('.undo-fab__main').first().click();
    await expect(row).toBeFocused();
    await expect(row).toHaveAttribute('aria-selected', 'true');
    await expect(page.locator('h1', { hasText: 'Google AI Studio quota warning' })).toBeVisible();
  });
}

for (const trigger of ['keyboard', 'reader'] as const) {
  test(`bulk Done through ${trigger} and undo keep the same selection`, async ({ page }) => {
    await page.keyboard.press('k');
    await page.keyboard.press('k');
    await page.keyboard.press('x');
    await page.keyboard.press('j');
    await page.keyboard.press('x');
    await page.keyboard.press('j');
    await page.keyboard.press('x');
    const bar = page.locator('.selection-tools');
    await expect(bar.getByText('3 selected', { exact: true })).toBeVisible();
    await expect(page.locator('.thread-toolbar__done')).toHaveText('Done (3)');
    if (trigger === 'keyboard') await page.keyboard.press('d');
    else await page.locator('.thread-toolbar__done').click();
    for (const id of ['th-1', 'th-6', 'th-7']) {
      await expect(page.locator(`[data-list-target-key="thread:${id}"]`)).toHaveCount(0);
    }
    await page.keyboard.press('j');
    await page.keyboard.press('z');
    await expect(bar.getByText('3 selected', { exact: true })).toBeVisible();
    await expect(page.locator('[data-list-target-key="thread:th-1"]')).toBeFocused();
    for (const id of ['th-1', 'th-6', 'th-7']) {
      await expect(page.locator(`[data-list-target-key="thread:${id}"]`)).toHaveAttribute('aria-selected', 'true');
    }
    await expect(page.locator('h1', { hasText: 'Staging login 500 after deployment' })).toBeVisible();
  });
}
