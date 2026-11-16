import type { Page } from '@playwright/test';
import { expect, test } from './mock-contract.fixture';

/*
 * Mail merge from the composer: a CSV list, a field from the picker, the
 * per-recipient preview with its skipped rows, the final confirm, and the
 * job in the Outbox. Runs at desktop and phone width.
 */

const CSV = 'Vorname;Nachname;E-Mail\nAnna;Meier;anna@example.test\nBen;;not-an-address\nCem;Kaya;cem@example.test\n';

async function openMergeComposer(page: Page) {
  await page.goto('/');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
  // The phone composes from its floating button; the desktop from the toolbar.
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const compose = (page.viewportSize()?.width ?? 1440) < 600
    ? page.locator('.fab')
    : page.getByRole('button', { name: 'New message', exact: true }).first();
  // A click during the first paint can land before the button is wired up.
  await expect(async () => {
    if (!(await composer.isVisible())) await compose.click({ timeout: 2_000 });
    await expect(composer).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 20_000 });
  await composer.getByLabel('More options', { exact: true }).click();
  await composer.getByTestId('compose-mail-merge').click();
  const source = page.getByTestId('mail-merge-source');
  await expect(source).toBeVisible();
  await source.getByTestId('mail-merge-file').setInputFiles({
    name: 'customers.csv',
    mimeType: 'text/csv',
    buffer: Buffer.from(CSV, 'utf8'),
  });
  await expect(source.getByText('3 rows', { exact: false })).toBeVisible();
  await expect(source.getByTestId('mail-merge-email-field')).toHaveValue('E-Mail');
  await source.getByTestId('mail-merge-source-apply').click();
  await expect(composer.getByTestId('mail-merge-bar')).toContainText('3 recipients from customers.csv');
  return composer;
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
  test(`mail merge previews each copy and starts a job, ${viewport.width}px`, async ({ page }) => {
    await page.setViewportSize(viewport);
    const composer = await openMergeComposer(page);

    const subject = composer.getByLabel('Subject', { exact: true });
    await subject.fill('Hello ');
    await subject.click();
    await subject.press('End');
    await composer.getByTestId('mail-merge-field-picker').selectOption('Vorname');
    await expect(subject).toHaveValue('Hello {{Vorname}}');
    await composer.locator('.ProseMirror').first().fill('Dear {{Vorname}} {{Nachname|friend}},');

    await composer.getByTestId('compose-merge-review').click();
    const review = page.getByTestId('mail-merge-review');
    await expect(review).toBeVisible();
    await expect(review).toContainText('2 of 3 rows in customers.csv will be sent.');
    await expect(review.getByTestId('mail-merge-position')).toHaveText('1 of 3');
    await expect(review.getByTestId('mail-merge-preview-subject')).toHaveText('Hello Anna');

    await review.getByRole('button', { name: 'Next recipient', exact: true }).click();
    await expect(review.getByTestId('mail-merge-position')).toHaveText('2 of 3');
    await expect(review).toContainText('No valid email address');
    await review.getByRole('button', { name: 'Next recipient', exact: true }).click();
    await expect(review.getByTestId('mail-merge-preview-subject')).toHaveText('Hello Cem');
    await expect(review.getByTestId('mail-merge-skipped')).toContainText('One row is skipped');

    // Nothing overflows sideways, phone included.
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow).toBeLessThanOrEqual(0);

    await review.getByTestId('mail-merge-delay').fill('30');
    await review.getByTestId('mail-merge-start').click();
    const confirm = page.getByRole('dialog', { name: 'Send 2 personalised messages?' });
    await expect(confirm).toBeVisible();
    await confirm.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(page.getByText('Mail merge started: 2 copies go out one by one.').first()).toBeVisible();

    await page.getByRole('button', { name: 'View progress', exact: true }).first().click();
    const jobs = page.getByTestId('mail-merge-jobs');
    await expect(jobs).toContainText('Hello {{Vorname}}');
    await expect(jobs).toContainText('1 of 2 sent');
    await jobs.getByRole('button', { name: 'Pause', exact: true }).click();
    await expect(jobs).toContainText('Paused');
  });
}

test('mail merge keeps the field picker and review reachable by keyboard', async ({ page }) => {
  const composer = await openMergeComposer(page);
  await composer.locator('.ProseMirror').first().fill('Hi');
  await page.keyboard.press('ControlOrMeta+Enter');
  const review = page.getByTestId('mail-merge-review');
  await expect(review).toBeVisible();
  await review.getByTestId('mail-merge-position').click();
  await page.keyboard.press('ArrowRight');
  await expect(review.getByTestId('mail-merge-position')).toHaveText('2 of 3');
  await page.keyboard.press('Escape');
  await expect(review).toBeHidden();
  await expect(composer.getByTestId('mail-merge-bar')).toBeVisible();
});

test('mail merge says when a list was read as Windows-1252, and refuses an oversized file', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await expect(async () => {
    if (!(await composer.isVisible())) await page.getByRole('button', { name: 'New message', exact: true }).first().click({ timeout: 2_000 });
    await expect(composer).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 20_000 });
  await composer.getByLabel('More options', { exact: true }).click();
  await composer.getByTestId('compose-mail-merge').click();
  const source = page.getByTestId('mail-merge-source');
  // "Jürgen" with ü as the single Windows-1252 byte 0xFC.
  await source.getByTestId('mail-merge-file').setInputFiles({
    name: 'excel.csv',
    mimeType: 'text/csv',
    buffer: Buffer.from([...Buffer.from('Name;E-Mail\nJ'), 0xfc, ...Buffer.from('rgen;j@example.test\n')]),
  });
  await expect(source.getByTestId('mail-merge-encoding-hint')).toContainText('Windows-1252');
  await expect(source).toContainText('Windows-1252');

  await source.getByTestId('mail-merge-file').setInputFiles({
    name: 'huge.csv',
    mimeType: 'text/csv',
    buffer: Buffer.alloc(5 * 1024 * 1024 + 1, 'a'),
  });
  await expect(source).toContainText('larger than 5 MB');
});
