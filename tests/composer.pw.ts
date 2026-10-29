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
        if (mode === 'new') await page.getByRole('button', { name: 'New message', exact: true }).click();
        else {
          // The fixture thread carries an unsent draft, and Reply resumes it.
          // Discard it first so this checks a fresh reply's layout.
          await page.getByRole('region', { name: 'Unsent drafts' }).getByRole('button', { name: 'Discard', exact: true }).click();
          await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
        }
        const composer = page.getByRole('dialog', { name: 'Compose email' });
        await expect(composer).toBeVisible();
        const writer = composer.locator('.ProseMirror').first();
        await writer.fill(mode === 'new' ? 'Hi Ada,\n\nThe release is ready for review.' : 'Thanks for the update. I will review the deployment today.');
        if (mode === 'new') {
          const recipient = composer.locator('.recipient-row input').first();
          await recipient.fill('ada@example.com');
          await recipient.press('Enter');
        }
        // Toasts expire on their own meanwhile, so a list taken up front goes
        // stale: dismiss whichever is still up until none is.
        const dismiss = page.getByRole('button', { name: 'Dismiss message', exact: true });
        await expect(async () => {
          if (await dismiss.count()) await dismiss.first().click({ timeout: 1_000 });
          await expect(dismiss).toHaveCount(0, { timeout: 0 });
        }).toPass();
        const signature = composer.locator('[data-composer-signature]');
        await expect(signature).toContainText('Kind regards');
        await expect.poll(async () => {
          const box = await composer.boundingBox();
          return box && Math.round(viewport.width - box.x - box.width);
        }).toBe(18);
        await expect.poll(async () => {
          const box = (await composer.boundingBox())!;
          return Math.round(viewport.height - box.y - box.height);
        }).toBe(18);
        await expect.poll(async () => (await composer.boundingBox())!.width).toBeCloseTo(620, 3);
        await expect.poll(async () => (await composer.boundingBox())!.height).toBeCloseTo(540, 3);
        if (mode === 'reply') {
          const history = composer.locator('[data-composer-history]');
          await expect(history).toContainText(/On .+ wrote:/);
          await expect(history.locator('p').first()).not.toContainText(/\d{1,2}:\d{2}:\d{2}/);
          const order = await composer.locator('.body').evaluate((body) => Array.from(body.children).map((element) => element.hasAttribute('data-composer-signature') ? 'signature' : element.hasAttribute('data-composer-history') ? 'history' : 'writing'));
          expect(order.slice(0, 3)).toEqual(['writing', 'signature', 'history']);
          const writerBox = (await writer.boundingBox())!;
          const signatureBox = (await signature.boundingBox())!;
          const historyBox = (await history.boundingBox())!;
          expect(writerBox.y + writerBox.height).toBeLessThanOrEqual(signatureBox.y);
          expect(signatureBox.y - writerBox.y - writerBox.height).toBeLessThan(32);
          expect(signatureBox.y + signatureBox.height).toBeLessThanOrEqual(historyBox.y);
        }
        const send = composer.getByRole('button', { name: 'Send', exact: true });
        const later = composer.getByRole('button', { name: 'Send later…', exact: true });
        await expect(send).toBeEnabled();
        await expect(later).toBeEnabled();
        const signatureBox = (await signature.boundingBox())!;
        const sendBox = (await send.boundingBox())!;
        expect(signatureBox.y + signatureBox.height).toBeLessThanOrEqual(sendBox.y);
        const box = (await composer.boundingBox())!;
        expect(sendBox.y + sendBox.height).toBeLessThan(box.y + box.height);
        await page.screenshot({ path: testInfo.outputPath(`${mode}-${theme}-${viewport.width}x${viewport.height}@2x.png`) });
        await send.focus();
        await page.keyboard.press('Tab');
        await expect(later).toBeFocused();
        await page.keyboard.press('Enter');
        const picker = page.getByRole('menu', { name: 'Send later…', exact: true });
        await expect(picker).toBeVisible();
        await expect(picker).toHaveCSS('opacity', '1');
        await expect(picker).toHaveClass('context-menu');
        await expect(later).toHaveAttribute('aria-haspopup', 'menu');
        const options = picker.getByRole('menuitem');
        await expect(options).toHaveCount(4);
        await expect(options.first()).toBeFocused();
        await expect.poll(async () => {
          const menuBox = (await picker.boundingBox())!;
          const triggerBox = (await later.boundingBox())!;
          return Math.abs(menuBox.y + menuBox.height - triggerBox.y);
        }).toBeLessThan(32);
        await page.keyboard.press('ArrowDown');
        await expect(options.nth(1)).toBeFocused();
        await page.keyboard.press('ArrowUp');
        await expect(options.first()).toBeFocused();
        await expect(later).toHaveAttribute('aria-expanded', 'true');
        await page.screenshot({ path: testInfo.outputPath(`${mode}-send-later-${theme}-${viewport.width}x${viewport.height}@2x.png`) });
        await page.keyboard.press('Escape');
        await expect(picker).toHaveCount(0);
        await expect(later).toBeFocused();
        await expect(later).toHaveAttribute('aria-expanded', 'false');
        await later.press('ArrowDown');
        await expect(picker).toBeVisible();
        await page.keyboard.press('End');
        await page.keyboard.press('Enter');
        const custom = page.getByRole('dialog', { name: 'Send later…', exact: true });
        await expect(custom).toBeVisible();
        await expect(custom.locator('input')).toHaveAttribute('step', '60');
        await page.keyboard.press('Escape');
        await expect(custom).toHaveCount(0);
        await expect(later).toBeFocused();
        await later.press('ArrowDown');
        await expect(options.first()).toBeFocused();
        await page.keyboard.press('Enter');
        await expect(picker).toHaveCount(0);
        await expect(composer.locator('.schedule-info')).toContainText('Scheduled');
        await expect(composer.locator('.split-button__primary')).toContainText('Schedule');
      });
    }
  }
}

test('signature switches in place and survives saving and reopening a reply draft', async ({ page }) => {
  await page.goto('/?mockSignatures=1');
  // Reply resumes the fixture thread's unsent draft; start from a fresh reply.
  await page.getByRole('region', { name: 'Unsent drafts' }).getByRole('button', { name: 'Discard', exact: true }).click();
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
  expect(box).toMatchObject({ x: 0, width: 390 });
  expect(Math.round(box.y + box.height)).toBe(844);
  // Fresh failure receipts reserve a lane above the full-screen composer.
  for (const toast of await page.locator('.toast').all()) {
    const receipt = (await toast.boundingBox())!;
    expect(receipt.y + receipt.height).toBeLessThanOrEqual(box.y);
  }
  await expect(composer.getByRole('button', { name: 'Maximise composer' })).toHaveCount(0);
  await composer.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(composer).toHaveCount(0);
  await expect(page.locator('.fab')).toBeVisible();
});

test('drag, resize, maximize and reopen keep the composer inside the window', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mockSignatures=1');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
  await page.getByRole('button', { name: 'New message', exact: true }).click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await expect(composer).toBeVisible();
  // Docking runs after the first layout; measure once the window has settled
  // against the bottom-right corner, not while it is still at its first frame.
  await expect.poll(async () => {
    const box = (await composer.boundingBox())!;
    return Math.round(1440 - box.x - box.width);
  }).toBeLessThan(40);
  await composer.locator('.ProseMirror').first().fill('Hi Ada,\n\nThe release is ready for review.');
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
  await page.mouse.move(resize.x + 102, resize.y + 92, { steps: 4 });
  await page.mouse.up();
  const resized = (await composer.boundingBox())!;
  expect(Math.round(resized.width)).toBe(520);
  expect(Math.round(resized.height)).toBe(500);
  const signature = (await composer.locator('[data-composer-signature]').boundingBox())!;
  const footer = (await composer.locator('.footer').boundingBox())!;
  expect(signature.y + signature.height).toBeLessThanOrEqual(footer.y);
  await composer.getByRole('button', { name: 'Cc/Bcc', exact: true }).click();
  await composer.locator('input[type="file"]').setInputFiles({
    name: 'release.txt', mimeType: 'text/plain', buffer: Buffer.from('Release notes'),
  });
  await expect(composer.locator('.compose-attachments')).toContainText('release.txt');
  const expandedFooter = (await composer.locator('.footer').boundingBox())!;
  expect(expandedFooter.y + expandedFooter.height).toBeLessThanOrEqual(resized.y + resized.height);
  await composer.getByRole('button', { name: 'Maximise composer' }).click();
  await expect.poll(async () => Math.round((await composer.boundingBox())!.width)).toBe(1404);
  await composer.getByRole('button', { name: 'Restore window' }).click();
  await expect.poll(async () => Math.round((await composer.boundingBox())!.width)).toBe(520);
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
  await expect.poll(async () => Math.round((await composer.boundingBox())!.x)).toBe(802);
});

test('a thread dragged from the list onto the composer arrives as an .eml attachment', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mockSignatures=1');
  const row = page.getByRole('option').filter({ hasText: 'Staging login 500 after deployment' }).first();
  await expect(row).toBeVisible();
  await page.getByRole('button', { name: 'New message', exact: true }).click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const writer = composer.locator('.ProseMirror').first();
  await expect(writer).toBeVisible();
  // Dropped on the editor on purpose: the composer must take it before the
  // editor pastes the drag as text.
  await row.dragTo(writer);
  await expect(composer.locator('.compose-attachments')).toContainText('Staging login 500 after deployment.eml');
  await expect(writer).toHaveText('');
  await expect(composer.locator('.compose-eml-drop')).toHaveCount(0);
});
