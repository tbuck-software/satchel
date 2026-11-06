import { expect, test } from './mock-contract.fixture';

test('tasks: reach the view by key, quick-add, complete, and make a task from a mail', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('.thread-row').first()).toBeVisible();

  // `g k` opens Tasks from the mailbox, as in Gmail. The first keys can land
  // while the reader is still loading the opened thread, so press until on.
  await expect(async () => {
    await page.keyboard.press('g');
    await page.keyboard.press('k');
    await expect(page).toHaveURL(/\/tasks$/, { timeout: 1_000 });
  }).toPass({ timeout: 15_000 });
  await expect(page.getByRole('heading', { name: 'Tasks', exact: true }).first()).toBeVisible();
  await expect(page.locator('.task-row__title', { hasText: 'Renew passport' })).toBeVisible();
  await expect(page.getByText('Overdue', { exact: true })).toBeVisible();

  const quickAdd = page.getByTestId('task-quick-add');
  await quickAdd.fill('Pay rent tomorrow');
  await expect(page.getByText(/“Pay rent”, due Tomorrow/)).toBeVisible();
  await quickAdd.press('Enter');
  const created = page.locator('.task-row', { hasText: 'Pay rent' });
  await expect(created).toBeVisible();
  await expect(created).toContainText('Tomorrow');

  // The first row has the keyboard; x completes it and offers Undo.
  await quickAdd.blur();
  await page.keyboard.press('x');
  await expect(page.getByText('Done: Renew passport').first()).toBeVisible();
  await expect(page.locator('.task-row__title', { hasText: 'Renew passport' })).toHaveCount(0);

  // A task made from a mail links back to it.
  await page.locator('.task-row', { hasText: 'Reply with the quarterly numbers' }).getByRole('button', { name: 'Open mail' }).click();
  await expect(page).toHaveURL(/\/thread\/th-1$/);

  await page.goto('/');
  await expect(page.locator('.thread-row').first()).toBeVisible();
  await page.locator('.thread-row').first().click();
  await page.keyboard.press('Shift+T');
  const dialog = page.getByRole('dialog', { name: 'New task' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId('task-title')).not.toHaveValue('');
  await expect(dialog.getByText('Made from a mail.')).toBeVisible();
  await dialog.getByTestId('task-save').click();
  await expect(page.getByText('Task added, linked to the mail').first()).toBeVisible();
});

test('tasks: work at phone width with lists as pills and the tab bar', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/tasks');
  await expect(page.locator('.task-row').first()).toBeVisible();
  await expect(page.locator('.mobile-tab-bar [data-tab="tasks"]')).toHaveAttribute('aria-current', 'page');

  const pills = page.getByRole('navigation', { name: 'Task lists' });
  await pills.getByRole('button', { name: /Work/ }).click();
  await expect(page.locator('.task-row__title', { hasText: 'Reply with the quarterly numbers' })).toBeVisible();
  await expect(page.locator('.task-row__title', { hasText: 'Renew passport' })).toHaveCount(0);

  const overflow = await page.evaluate(() => document.scrollingElement!.scrollWidth - document.scrollingElement!.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);

  // The check is a touch target, not a dot.
  const check = await page.getByTestId('task-check').first().boundingBox();
  expect(check!.width).toBeGreaterThanOrEqual(44);
  expect(check!.height).toBeGreaterThanOrEqual(44);
});
