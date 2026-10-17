import { expect, test } from './mock-contract.fixture';

test('mobile mailbox controls stay open until a destination is selected', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Menu', exact: true }).click();
  const drawer = page.locator('.mobile-sidebar-panel');
  await expect(drawer).toBeVisible();
  await drawer.getByRole('button', { name: 'Filter mailboxes' }).click();
  const filter = page.getByRole('menu', { name: 'Filter mailboxes' });
  await expect(filter).toBeVisible();
  await expect(drawer).toBeVisible();
  await filter.getByRole('menuitemcheckbox').nth(1).click();
  await expect(drawer).not.toBeVisible();

  await page.getByRole('button', { name: 'Menu', exact: true }).click();
  await drawer.getByRole('button', { name: 'Filter mailboxes' }).click();
  await filter.getByRole('menuitemcheckbox', { name: 'All accounts' }).click();
  await expect(drawer).not.toBeVisible();
  await expect(page.locator('.pane-header')).toContainText('All accounts');
});

test('saved wide panes switch to a reachable single-pane layout and back', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('satchel.ui.sidebarWidth', '420');
    localStorage.setItem('satchel.ui.threadListWidth', '760');
  });
  await page.setViewportSize({ width: 1100, height: 900 });
  await page.goto('/');
  const inbox = page.locator('.inbox-layout');
  await expect(inbox).toHaveClass(/inbox-layout--single/);
  expect(await inbox.evaluate((element) => element.scrollWidth <= element.clientWidth + 1)).toBe(true);
  await expect(page.locator('.detail-pane')).toHaveCount(0);

  await page.setViewportSize({ width: 1600, height: 900 });
  await expect(inbox).not.toHaveClass(/inbox-layout--single/);
  await expect(page.locator('.detail-pane')).toBeVisible();
  expect(await inbox.evaluate((element) => element.scrollWidth <= element.clientWidth + 1)).toBe(true);
});

for (const width of [390, 900]) {
  test(`search at ${width}px returns to the same results after opening a conversation`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await page.goto('/search?q=Google&global=1');
    await page.getByRole('heading', { name: 'Search', exact: true }).waitFor();
    const query = page.getByRole('searchbox');
    await expect(query).toHaveValue('Google');
    const hit = page.locator('.search-scroll .result-row').first();
    await expect(hit).toBeVisible();
    await hit.click();
    await expect(query).not.toBeVisible();
    await page.getByRole('button', { name: 'Back', exact: true }).click();
    await expect(query).toBeVisible();
    await expect(query).toHaveValue('Google');
    await expect(page).toHaveURL(/global=1/);
    await hit.click();
    await expect(query).not.toBeVisible();
    await page.goBack();
    await expect(query).toBeVisible();
    await expect(query).toHaveValue('Google');
  });
}
