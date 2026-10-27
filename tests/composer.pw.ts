import { expect, test } from './mock-contract.fixture';

test.use({ deviceScaleFactor: 2 });

for (const theme of ['dark', 'light'] as const) {
  for (const viewport of [{ width: 1440, height: 900 }, { width: 1100, height: 700 }]) {
    for (const mode of ['new', 'reply'] as const) {
      test(`${mode} docks bottom-right, ${theme}, ${viewport.width}x${viewport.height}`, async ({ page }, testInfo) => {
        await page.setViewportSize(viewport);
        await page.addInitScript((value) => localStorage.setItem('satchel.theme', value), theme);
        await page.goto('/?mockSignatures=1');
        await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
        if (mode === 'new') await page.keyboard.press('n');
        else await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
        const composer = page.getByRole('dialog', { name: 'Compose email' });
        await expect(composer).toBeVisible();
        const writer = composer.locator('.ProseMirror').first();
        await writer.fill(mode === 'new' ? 'Hi Ada,\n\nThe release is ready for review.' : 'Thanks for the update. I will review the deployment today.');
        const signature = composer.locator('[data-composer-signature]');
        await expect(signature).toContainText('Kind regards');
        await expect.poll(async () => {
          const box = await composer.boundingBox();
          return box && Math.round(viewport.width - box.x - box.width);
        }).toBe(18);
        const box = (await composer.boundingBox())!;
        expect(Math.round(viewport.height - box.y - box.height)).toBe(18);
        expect(box.width).toBeCloseTo(760, 3);
        expect(box.height).toBeCloseTo(650, 3);
        if (mode === 'reply') {
          const history = composer.locator('[data-composer-history]');
          await expect(history).toContainText(/On .+ wrote:/);
          const order = await composer.locator('.body').evaluate((body) => Array.from(body.children).map((element) => element.hasAttribute('data-composer-signature') ? 'signature' : element.hasAttribute('data-composer-history') ? 'history' : 'writing'));
          expect(order.slice(0, 3)).toEqual(['writing', 'signature', 'history']);
          const writerBox = (await writer.boundingBox())!;
          const signatureBox = (await signature.boundingBox())!;
          const historyBox = (await history.boundingBox())!;
          expect(writerBox.y + writerBox.height).toBeLessThanOrEqual(signatureBox.y);
          expect(signatureBox.y - writerBox.y - writerBox.height).toBeLessThan(32);
          expect(signatureBox.y + signatureBox.height).toBeLessThanOrEqual(historyBox.y);
        }
        await page.screenshot({ path: testInfo.outputPath(`${mode}-${theme}-${viewport.width}x${viewport.height}@2x.png`) });
      });
    }
  }
}

test('signature switches in place and survives saving and reopening a reply draft', async ({ page }) => {
  await page.goto('/?mockSignatures=1');
  await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
  let composer = page.getByRole('dialog', { name: 'Compose email' });
  await composer.locator('.ProseMirror').first().fill('My answer for the saved draft.');
  await expect(composer.locator('[data-composer-signature]')).toContainText('Kind regards');
  await composer.getByRole('button', { name: 'Default', exact: true }).click();
  await page.getByRole('menuitemradio', { name: 'Brief', exact: true }).click();
  await expect(composer.locator('[data-composer-signature]')).toContainText('Thanks,');
  await expect(composer.locator('[data-composer-signature]')).not.toContainText('Kind regards');
  await composer.getByRole('button', { name: 'Close composer' }).click();
  await expect(composer).toHaveCount(0);
  await page.keyboard.press('g');
  await page.keyboard.press('d');
  await page.getByRole('listitem').filter({ hasText: 'My answer for the saved draft.' }).locator('.row__open').dblclick();
  composer = page.getByRole('dialog', { name: 'Compose email' });
  await expect(composer.locator('.ProseMirror').first()).toHaveText('My answer for the saved draft.');
  await expect(composer.locator('[data-composer-signature]')).toContainText('Thanks,');
  await expect(composer.locator('[data-composer-history]')).toContainText(/On .+ wrote:/);
});

test('phone ComposeFab opens a full-screen composer and returns to the mailbox', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?mockSignatures=1');
  await page.locator('.fab').click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await expect(composer).toBeVisible();
  const box = (await composer.boundingBox())!;
  expect(box).toMatchObject({ x: 0, y: 0, width: 390, height: 844 });
  await expect(composer.getByRole('button', { name: 'Maximise composer' })).toHaveCount(0);
  await composer.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(composer).toHaveCount(0);
  await expect(page.locator('.fab')).toBeVisible();
});

test('drag, resize, maximize and reopen keep the composer inside the window', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
  await page.keyboard.press('n');
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await expect(composer).toBeVisible();
  const initial = (await composer.boundingBox())!;
  const chrome = (await composer.locator('.composer-chrome').boundingBox())!;
  await page.mouse.move(chrome.x + 80, chrome.y + chrome.height / 2);
  await page.mouse.down();
  await page.mouse.move(chrome.x - 100, chrome.y - 60, { steps: 4 });
  await page.mouse.up();
  const moved = (await composer.boundingBox())!;
  expect(moved.x).toBeLessThan(initial.x);
  expect(moved.y).toBeLessThan(initial.y);
  const resize = (await composer.locator('.composer-resize-zone--nw').boundingBox())!;
  await page.mouse.move(resize.x + 2, resize.y + 2);
  await page.mouse.down();
  await page.mouse.move(resize.x + 62, resize.y + 42, { steps: 4 });
  await page.mouse.up();
  const resized = (await composer.boundingBox())!;
  expect(Math.round(resized.width)).toBe(700);
  expect(Math.round(resized.height)).toBe(610);
  await composer.getByRole('button', { name: 'Maximise composer' }).click();
  await expect.poll(async () => Math.round((await composer.boundingBox())!.width)).toBe(1404);
  await composer.getByRole('button', { name: 'Restore window' }).click();
  await expect.poll(async () => Math.round((await composer.boundingBox())!.width)).toBe(700);
  await page.setViewportSize({ width: 800, height: 500 });
  await expect.poll(async () => {
    const box = (await composer.boundingBox())!;
    return Math.round(box.y + box.height);
  }).toBeLessThanOrEqual(500);
  const fit = (await composer.boundingBox())!;
  expect(fit.x).toBeGreaterThanOrEqual(0);
  expect(fit.y).toBeGreaterThanOrEqual(0);
  expect(fit.x + fit.width).toBeLessThanOrEqual(800.5);
  expect(fit.y + fit.height).toBeLessThanOrEqual(500.5);
  await composer.getByRole('button', { name: 'Close composer' }).click();
  await expect(composer).toHaveCount(0);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.keyboard.press('n');
  await expect(composer).toBeVisible();
  await expect.poll(async () => Math.round((await composer.boundingBox())!.x)).toBe(662);
});
