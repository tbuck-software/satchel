import type { Locator, Page } from '@playwright/test';
import { expect, test } from './mock-contract.fixture';

/*
 * Outlook's drags onto the composer: mail from any list becomes an `.eml`
 * attachment, a contact a recipient, a chip moves between To, Cc and Bcc.
 *
 * These run in Chromium and WebKit. Neither is the built app: Tauri's own
 * drag-drop handler sits in front of WKWebView there, and only the built
 * `.app` shows whether a drag reaches the page (see `utils::navigation::guard`).
 */

test.use({ viewport: { width: 1440, height: 900 } });

async function openNewMessage(page: Page) {
  await page.getByRole('button', { name: 'New message', exact: true }).click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const writer = composer.locator('.ProseMirror').first();
  // The composer is a chunk of its own, loaded on the first open.
  await expect(writer).toBeVisible({ timeout: 30_000 });
  return { composer, writer };
}

async function expectEmlAttached(composer: Locator, writer: Locator) {
  await expect(composer.locator('.compose-attachments')).toContainText(/\.eml/);
  // The composer took the drag before the editor could paste it as text.
  await expect(writer).toHaveText('');
  await expect(composer.locator('.compose-eml-drop')).toHaveCount(0);
}

test('a hit on the search page drags onto the composer as an .eml', async ({ page }) => {
  await page.goto('/search?q=Release%20notes&global=1');
  const hit = page.locator('.result-row').filter({ hasText: 'Release notes for 4.2' }).first();
  // A cold dev server transforms the search page on first request.
  await expect(hit).toBeVisible({ timeout: 30_000 });
  const { composer, writer } = await openNewMessage(page);

  await hit.dragTo(writer);

  await expectEmlAttached(composer, writer);
  await expect(composer.locator('.compose-attachments')).toContainText('Release notes for 4.2');
});

test('a hit of the inline list search drags onto the composer as an .eml', async ({ page }) => {
  await page.goto('/');
  const search = page.getByPlaceholder(/^Search Inbox/);
  await search.fill('Staging');
  const hit = page.getByRole('option').filter({ hasText: 'Staging login 500' }).first();
  await expect(hit).toBeVisible({ timeout: 30_000 });
  const { composer, writer } = await openNewMessage(page);

  await hit.dragTo(writer);

  await expectEmlAttached(composer, writer);
});

test('one message of an open thread drags onto the reply as an .eml', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('option').filter({ hasText: 'Staging login 500 after deployment' }).first().click();
  await page.getByRole('region', { name: 'Unsent drafts' }).getByRole('button', { name: 'Discard', exact: true }).click();
  const header = page.locator('.message-surface .message-header').first();
  await expect(header).toBeVisible();
  await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const writer = composer.locator('.ProseMirror').first();
  await expect(writer).toBeVisible();
  await writer.fill('');

  await header.dragTo(writer);

  await expect(composer.locator('.compose-attachments')).toContainText(/\.eml/);
  await expect(writer).toHaveText('');
});

test.describe('recipients', () => {
  async function firstContact(page: Page) {
    await page.goto('/contacts');
    const row = page.locator('.contact-row').first();
    await expect(row).toBeVisible();
    const email = (await row.locator('.copy span').first().innerText()).trim();
    return { row, email: email.includes('@') ? email : (await row.locator('.copy strong').innerText()).trim() };
  }

  test('a contact dropped on To becomes a To recipient', async ({ page }) => {
    const { row, email } = await firstContact(page);
    const { composer } = await openNewMessage(page);

    await row.dragTo(composer.getByRole('combobox', { name: 'To', exact: true }));

    const chips = composer.locator('.recipient-row .chip');
    await expect(chips).toHaveCount(1);
    expect(await chips.first().getAttribute('title')).toContain(email);
  });

  test('a contact dropped on Cc goes to Cc, and on the body to To', async ({ page }) => {
    const { row } = await firstContact(page);
    const { composer, writer } = await openNewMessage(page);
    await composer.getByRole('button', { name: 'Cc/Bcc', exact: true }).click();
    const cc = composer.getByRole('combobox', { name: 'Cc', exact: true });

    await row.dragTo(cc);
    await expect(composer.locator('.compose-fold .chip')).toHaveCount(1);
    await expect(composer.locator('.recipient-row .chip')).toHaveCount(0);

    await page.locator('.contact-row').nth(1).dragTo(writer);
    await expect(composer.locator('.recipient-row .chip')).toHaveCount(1);
    await expect(writer).toHaveText('');
  });

  test('a chip dragged from To to Cc moves there', async ({ page }) => {
    await page.goto('/');
    const { composer } = await openNewMessage(page);
    const to = composer.getByRole('combobox', { name: 'To', exact: true });
    await to.fill('ada@example.test');
    await to.press('Enter');
    await composer.getByRole('button', { name: 'Cc/Bcc', exact: true }).click();
    const chip = composer.locator('.recipient-row .chip').filter({ hasText: 'ada@example.test' });
    await expect(chip).toBeVisible();

    await chip.dragTo(composer.getByRole('combobox', { name: 'Cc', exact: true }));

    await expect(composer.locator('.recipient-row .chip')).toHaveCount(0);
    const cc = composer.locator('.compose-fold .chip');
    await expect(cc).toHaveCount(1);
    await expect(cc.first()).toHaveAttribute('title', 'ada@example.test');
  });

  test('a thread dropped on a recipient field is attached, not added as a recipient', async ({ page }) => {
    await page.goto('/');
    const row = page.getByRole('option').filter({ hasText: 'Staging login 500 after deployment' }).first();
    await expect(row).toBeVisible();
    const { composer } = await openNewMessage(page);

    await row.dragTo(composer.getByRole('combobox', { name: 'To', exact: true }));

    await expect(composer.locator('.compose-attachments')).toContainText(/\.eml/);
    await expect(composer.locator('.recipient-row .chip')).toHaveCount(0);
    await expect(composer.getByRole('combobox', { name: 'To', exact: true })).toHaveValue('');
  });
});
