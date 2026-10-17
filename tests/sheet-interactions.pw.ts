import { expect, test } from './mock-contract.fixture';

for (const width of [1440, 900]) {
  test(`closes message details with the X button at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await page.getByText('Staging login 500 after deployment', { exact: true }).first().click();
    await page.locator('.recipient-summary:visible').first().click();

    const details = page.getByRole('dialog', { name: 'Message details', exact: true });
    await expect(details).toBeVisible();
    await details.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(details).toHaveCount(0);
  });
}

test('still dismisses message details by dragging the sheet content down', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 900 });
  await page.goto('/');
  await page.getByText('Staging login 500 after deployment', { exact: true }).first().click();
  await page.locator('.recipient-summary:visible').first().click();

  const details = page.getByRole('dialog', { name: 'Message details', exact: true });
  const title = details.locator('.recipient-details__title');
  await expect(details).toBeVisible();
  // The sheet is visible while it is still sliding up. Start the gesture only
  // after its opening transition has settled, at the title's final position.
  await expect(details.locator('..')).not.toHaveClass(/sheet-rise-enter-active/);
  await title.hover();
  const bounds = await title.boundingBox();
  expect(bounds).not.toBeNull();
  const x = bounds!.x + bounds!.width / 2;
  const y = bounds!.y + bounds!.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + 160, { steps: 8 });
  await page.mouse.up();
  await expect(details).toHaveCount(0);
});
