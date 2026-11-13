import type { Locator, Page } from '@playwright/test';
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

test('editable composer lines stay close to the text height', async ({ page }) => {
  // WebKit on macOS draws the caret over the whole line box, not the glyphs:
  // at 1.68 it stood half a line taller than the letters. Linux WebKit and
  // Chromium draw it at font height, so the line box is the part to check.
  await page.goto('/');
  await page.getByRole('region', { name: 'Unsent drafts' }).getByRole('button', { name: 'Discard', exact: true }).click();
  await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const writer = composer.locator('.ProseMirror').first();
  await writer.fill('One line.');
  for (const editor of [writer, composer.locator('[data-composer-history] .ProseMirror')]) {
    const ratio = await editor.locator('p').first().evaluate((line) =>
      line.getBoundingClientRect().height / parseFloat(getComputedStyle(line).fontSize));
    expect(ratio).toBeLessThanOrEqual(1.4);
  }
});

test('lists in the composer show their markers', async ({ page }) => {
  // Tailwind's preflight sets `list-style: none` on every ul/ol in the app.
  await page.goto('/');
  await page.getByRole('button', { name: 'New message', exact: true }).click();
  const writer = page.getByRole('dialog', { name: 'Compose email' }).locator('.ProseMirror').first();
  await writer.click();
  await page.keyboard.type('- Tenant-ID');
  await page.keyboard.press('Enter');
  await page.keyboard.press('Enter');
  await page.keyboard.type('1. Client-ID');
  await expect(writer.locator('ul')).toHaveCSS('list-style-type', 'disc');
  await expect(writer.locator('ol')).toHaveCSS('list-style-type', 'decimal');
});

test('a draft sent or discarded elsewhere still lets the composer close', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'New message', exact: true }).click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const recipient = composer.locator('.recipient-row input').first();
  await recipient.fill('ada@example.com');
  await recipient.press('Enter');
  await composer.locator('.ProseMirror').first().fill('First version.');
  // Saved and reopened, the editor holds the draft's id and revision.
  await composer.getByRole('button', { name: 'Close composer' }).click();
  await page.locator('.toast__action').filter({ hasText: 'Continue editing' }).click();
  await expect(composer.locator('.ProseMirror').first()).toHaveText('First version.');
  // Another window or app sends the draft; this editor still holds it.
  await page.evaluate(async (mockModule) => {
    type Mock = { sendCommand: (command: string, payload?: Record<string, unknown>) => Promise<unknown> };
    const backend = await import(/* @vite-ignore */ mockModule) as Mock;
    const drafts = await backend.sendCommand('list_drafts') as { id: string }[];
    for (const draft of drafts) await backend.sendCommand('delete_draft', { id: draft.id });
  }, '/src/services/backend/mock.ts');
  await composer.locator('.ProseMirror').first().fill('Written after the draft went away.');

  await composer.getByRole('button', { name: 'Close composer' }).click();
  const question = page.getByRole('dialog', { name: 'Close without saving?' });
  await expect(question).toBeVisible();
  await question.getByRole('button', { name: 'Cancel' }).click();
  await expect(composer).toBeVisible();

  // The banner's way out sits inside the composer, not past its edge.
  const composerBox = (await composer.boundingBox())!;
  for (const name of ['Save as new draft', 'Discard my changes']) {
    const box = (await composer.getByRole('button', { name, exact: true }).boundingBox())!;
    expect(box.x + box.width).toBeLessThanOrEqual(composerBox.x + composerBox.width);
  }

  await composer.getByRole('button', { name: 'Close composer' }).click();
  await question.getByRole('button', { name: 'Close without saving' }).click();
  await expect(composer).toHaveCount(0);
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

test('phone composer scrolls fields and body as one column and keeps the typed line above Send', async ({ page }) => {
  await page.setViewportSize({ width: 393, height: 852 });
  await page.goto('/?mockSignatures=1&mockOutbox=0');
  await page.locator('.fab').click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await expect(composer).toBeVisible();
  // From folds under To with Cc and Bcc.
  const summary = composer.getByRole('button', { name: 'Cc/Bcc, From: tom@satchel.dev' });
  await expect(summary).toBeVisible();
  const fold = composer.getByTestId('compose-fold');
  expect((await fold.boundingBox())?.height).toBe(0);
  // What keyboardInset does when iOS raises the keyboard.
  await page.evaluate(() => {
    document.documentElement.style.setProperty('--satchel-safe-area-bottom', '336px');
    document.documentElement.style.setProperty('--satchel-keyboard-inset', '336px');
  });
  const editor = composer.locator('.ProseMirror').first();
  await editor.click();
  for (let line = 1; line <= 14; line += 1) {
    await page.keyboard.type(`Line ${line}: what changed in this release and why it matters.`);
    await page.keyboard.press('Enter');
  }
  await page.keyboard.type('The line being typed');
  const column = composer.getByTestId('compose-scroll');
  const footer = (await composer.locator('.footer').boundingBox())!;
  await expect.poll(async () => {
    const line = (await editor.locator('p').last().boundingBox())!;
    return line.y + line.height <= footer.y;
  }).toBe(true);
  // The fields went up with the text instead of staying pinned over it.
  const toRow = (await composer.locator('.recipient-row').boundingBox())!;
  const columnBox = (await column.boundingBox())!;
  expect(toRow.y + toRow.height).toBeLessThanOrEqual(columnBox.y);
  // ...and come back on scrolling up.
  await column.evaluate((element) => element.scrollTo(0, 0));
  await expect(composer.locator('.recipient-row')).toBeInViewport();
  await summary.click();
  await expect(composer.getByRole('combobox', { name: 'Cc', exact: true })).toBeFocused();
  await expect.poll(async () => (await fold.boundingBox())?.height ?? 0).toBeGreaterThan(100);
  await expect(composer.getByRole('combobox', { name: 'Sender' })).toBeInViewport();
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
  // Grab the zone at its centre: its outer pixels lie outside the window's
  // rounded corner, and Chromium 156 clips hit-testing to that corner.
  const grabX = resize.x + resize.width / 2;
  const grabY = resize.y + resize.height / 2;
  await page.mouse.move(grabX, grabY);
  await page.mouse.down();
  await page.mouse.move(grabX + 100, grabY + 90, { steps: 4 });
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


test.describe('recipient field accessibility', () => {
  test.use({ hasTouch: true });

  for (const activation of ['semantic click', 'mouse', 'keyboard', 'touch'] as const) {
    test(`names populated recipient inputs and selects suggestions by ${activation}`, async ({ page }) => {
      await page.goto('/');
      await page.getByRole('button', { name: 'New message', exact: true }).click();
      const composer = page.getByRole('dialog', { name: 'Compose email' });
      await composer.getByRole('button', { name: 'Cc/Bcc', exact: true }).click();
      for (const name of ['To', 'Cc', 'Bcc']) {
        const input = composer.getByRole('combobox', { name, exact: true });
        await input.fill(`${name.toLowerCase()}@example.com`);
        await input.press('Enter');
        await expect(input).toHaveAccessibleName(name);
        await expect(input).toHaveAttribute('placeholder', '');
        const label = composer.locator('label').filter({ hasText: new RegExp(`^${name}$`) });
        expect(await label.getAttribute('for')).toBe(await input.getAttribute('id'));
        await label.click();
        await expect(input).toBeFocused();
      }
      const input = composer.getByRole('combobox', { name: 'To', exact: true });
      await input.fill('example');
      const option = composer.getByRole('listbox').getByRole('option').first();
      await expect(option).toBeVisible();
      const email = await option.locator('small').innerText();
      if (activation === 'semantic click') await option.dispatchEvent('click');
      else if (activation === 'mouse') await option.click();
      else if (activation === 'touch') await option.tap();
      else await input.press('Enter');
      const chips = composer.locator('.recipient-row .chip');
      await expect(chips).toHaveCount(2);
      expect(await chips.last().getAttribute('title')).toContain(email);
      await expect(input).toHaveValue('');
      await expect(composer.getByRole('listbox').getByRole('option')).toHaveCount(0);
    });
  }
});

test('requires a future send time even when it expires with the composer open', async ({ page }) => {
  await page.clock.setFixedTime(new Date('2030-10-29T12:00:00'));
  await page.goto('/');
  await page.getByRole('button', { name: 'New message', exact: true }).click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const to = composer.getByRole('combobox', { name: 'To', exact: true });
  await to.fill('recipient@example.com');
  await to.press('Enter');
  await composer.locator('.ProseMirror').first().fill('Send only at the selected time.');
  await composer.getByRole('button', { name: 'Send later…', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Pick date & time…', exact: true }).click();
  const picker = page.getByRole('dialog', { name: 'Send later…', exact: true });
  await picker.locator('input').fill('2030-10-29T11:59');
  await picker.getByRole('button', { name: 'Set', exact: true }).click();
  await expect(picker.getByRole('alert')).toContainText('future');
  await expect(picker.locator('input')).toHaveAttribute('aria-invalid', 'true');
  await picker.locator('input').fill('2030-10-29T12:01');
  await picker.getByRole('button', { name: 'Set', exact: true }).click();
  await expect(picker).toHaveCount(0);
  const schedule = composer.getByRole('button', { name: 'Schedule', exact: true });
  await expect(schedule).toBeEnabled();
  await page.clock.setFixedTime(new Date('2030-10-29T12:01:00'));
  await expect(schedule).toBeDisabled();
  await expect(composer.locator('[data-testid="compose-send-wrap"]')).toHaveAttribute('title', /future/);
  const later = composer.getByRole('button', { name: 'Send later…', exact: true });
  await expect(later).toBeEnabled();
  await later.click();
  await page.getByRole('menu', { name: 'Send later…', exact: true }).getByRole('menuitem').first().click();
  await expect(schedule).toBeEnabled();
  await composer.locator('.schedule-info').getByRole('button').click();
  await expect(composer.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
});

/** ⌘⌥ on macOS; Alt+Shift elsewhere, where Ctrl+Alt+T and Ctrl+Alt+arrows belong to the desktop. */
async function tableChord(page: Page): Promise<string> {
  const apple = await page.evaluate(() => /mac|iphone|ipad|ipod/i.test(navigator.platform));
  return apple ? 'Meta+Alt' : 'Alt+Shift';
}

/**
 * Keydowns dispatched in one task, as fast as no keyboard could: the second
 * arrives before the editor has rendered anything for the first.
 */
/** What a dispatched keydown needs; plain data, so it crosses into the page. */
interface KeyDown {
  key: string;
  code: string;
  keyCode: number;
  altKey?: boolean;
  shiftKey?: boolean;
  metaKey?: boolean;
}

async function pressAtOnce(editor: Locator, keys: KeyDown[]) {
  await editor.evaluate((element, events) => {
    for (const init of events) {
      element.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }));
    }
  }, keys);
}

const ALT_F10: KeyDown = { key: 'F10', code: 'F10', keyCode: 121, altKey: true };

function insertKeys(chord: string): KeyDown {
  return chord === 'Meta+Alt'
    ? { key: 't', code: 'KeyT', keyCode: 84, metaKey: true, altKey: true }
    : { key: 'T', code: 'KeyT', keyCode: 84, altKey: true, shiftKey: true };
}

test('a table is inserted, filled and reshaped from the keyboard alone', async ({ page }) => {
  await page.goto('/');
  const chord = await tableChord(page);
  await page.getByRole('button', { name: 'New message', exact: true }).click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const editor = composer.locator('.ProseMirror').first();
  const toolbar = composer.getByRole('toolbar', { name: 'Table' });
  const firstTool = toolbar.getByRole('button', { name: 'Insert row above' });
  await editor.click();
  await page.keyboard.type('Figures:');

  // Alt+F10 straight after the insert, before the menu has shown, still lands.
  await pressAtOnce(editor, [insertKeys(chord), ALT_F10]);
  await expect(firstTool).toBeFocused();
  const table = editor.locator('table');
  await expect(table.locator('tr')).toHaveCount(3);
  await expect(table.locator('tr').first().locator('th')).toHaveCount(3);

  // Escape leaves the toolbar for the cell, not the composer.
  await page.keyboard.press('Escape');
  await expect(editor).toBeFocused();
  await expect(composer).toBeVisible();
  await page.keyboard.type('Quarter');
  // …and so does the insert chord right after Tab, inside a table.
  await pressAtOnce(editor, [{ key: 'Tab', code: 'Tab', keyCode: 9 }, insertKeys(chord)]);
  await expect(firstTool).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(editor).toBeFocused();
  await page.keyboard.type('Revenue');
  await expect(table.locator('th').nth(1)).toHaveText('Revenue');
  await expect(editor.locator('p').first()).toHaveText('Figures:');

  // Back from another field with a cell's text selected, the menu shows only
  // once the editor has looked at the selection again; Alt+F10 at once lands.
  await page.keyboard.press('Shift+Tab');
  await composer.getByRole('textbox', { name: 'Subject' }).focus();
  await expect(toolbar).toBeHidden();
  await editor.evaluate((element, init) => {
    (element as HTMLElement).focus();
    element.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }));
  }, ALT_F10);
  await expect(firstTool).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(editor).toBeFocused();

  await page.keyboard.press(`${chord}+ArrowDown`);
  await expect(table.locator('tr')).toHaveCount(4);

  // Arrows walk the toolbar; Enter runs a tool and hands the keys back.
  await page.keyboard.press('Alt+F10');
  await expect(firstTool).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowRight');
  await expect(toolbar.getByRole('button', { name: 'Insert column right' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(table.locator('tr').first().locator('th')).toHaveCount(4);
  await expect(editor).toBeFocused();

  await page.keyboard.press('Alt+F10');
  await expect(firstTool).toBeFocused();
  await page.keyboard.press('End');
  await expect(toolbar.getByRole('button', { name: 'Delete table' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(editor.locator('table')).toHaveCount(0);
  await expect(toolbar).toBeHidden();
});

test('a wide table scrolls inside itself on a phone instead of widening the composer', async ({ page }) => {
  await page.setViewportSize({ width: 393, height: 852 });
  await page.goto('/?mockOutbox=0');
  const chord = await tableChord(page);
  await page.locator('.fab').click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const editor = composer.locator('.ProseMirror').first();
  await editor.click();
  await page.keyboard.press(`${chord}+T`);
  for (let column = 0; column < 5; column += 1) await page.keyboard.press(`${chord}+ArrowRight`);
  await expect(editor.locator('tr').first().locator('th')).toHaveCount(8);
  await page.keyboard.type('A heading long enough to need the room');

  const wrapper = editor.locator('.tableWrapper');
  const overflow = await wrapper.evaluate((element) => element.scrollWidth - element.clientWidth);
  expect(overflow).toBeGreaterThan(0);
  const column = composer.getByTestId('compose-scroll');
  expect(await column.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(0);
});
