import { expect, test } from './mock-contract.fixture';

test('nightly updates keep the same app and announce a download without moving the sidebar', async ({ page }) => {
  await page.goto('/settings?section=about');
  const toggle = page.getByRole('switch', { name: 'Nightly updates', exact: true });
  await expect(toggle).toBeEnabled();
  await expect(toggle).not.toBeChecked();
  await page.locator('.about-pane .toggle-switch').click();
  await expect(toggle).toBeChecked();
  const footer = page.locator('.sidebar-footer').first();
  const restingFooter = await footer.boundingBox();
  await expect(footer.getByRole('button', { name: 'Update available', exact: true })).toHaveCount(0);

  await page.evaluate(async () => {
    const modulePath = '/src/stores/appUpdate.ts';
    const { useAppUpdateStore } = await import(modulePath);
    const update = useAppUpdateStore();
    update.offerUpdate({ version: '0.1.10-nightly.42', download: async () => {}, restart: async () => {} });
    await update.downloadUpdate();
  });

  const toast = page.locator('.toast').filter({ hasText: 'Satchel 0.1.10-nightly.42 downloaded' });
  await expect(toast.getByRole('button', { name: 'Restart now', exact: true })).toBeVisible();
  expect(await footer.boundingBox()).toEqual(restingFooter);
  await toast.getByRole('button', { name: 'Dismiss message', exact: true }).click();
  await expect(toast).toHaveCount(0);
  await expect(page.locator('.about-pane').getByRole('button', { name: 'Restart to update', exact: true })).toBeVisible();
  // The sidebar offers it too, beside Settings, only now that there is one.
  await expect(footer.getByRole('button', { name: 'Restart to update', exact: true })).toBeVisible();
  expect(await footer.boundingBox()).toEqual(restingFooter);
});

test('sync activity keeps the sidebar geometry stable', async ({ page }) => {
  await page.goto('/settings');
  const sidebar = page.locator('aside').first();
  const footer = sidebar.locator('.sidebar-footer');
  await expect(footer.getByRole('button', { name: 'Settings', exact: true })).toHaveAttribute('aria-current', 'page');
  const button = sidebar.locator('.sidebar-brand .sync-button');
  await expect(button).toBeVisible();
  const restingButton = await button.boundingBox();
  const restingFooter = await footer.boundingBox();
  const restingContent = await sidebar.locator('.sidebar-scroll').boundingBox();

  for (const state of [
    { syncing: true, online: true },
    { syncing: false, online: true },
    { syncing: false, online: false },
    { syncing: false, online: true },
  ]) {
    await page.evaluate(async (nextState) => {
      const root = document.querySelector('#app') as Element & {
        __vue_app__: { config: { globalProperties: {
          $pinia: { _s: Map<string, { $patch: (state: typeof nextState) => void }> };
        } } };
      };
      root.__vue_app__.config.globalProperties.$pinia._s.get('connection')!.$patch(nextState);
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    }, state);
    expect(await footer.boundingBox()).toEqual(restingFooter);
    expect(await sidebar.locator('.sidebar-scroll').boundingBox()).toEqual(restingContent);
    expect(await button.boundingBox()).toEqual(restingButton);
    await expect(button).toHaveAttribute('aria-busy', String(state.syncing));
    // Existing failures stay reachable while a sync runs or the app is offline.
    await expect(button).toBeEnabled();
  }
  await button.click();
  const failures = page.getByRole('dialog', { name: 'Needs attention', exact: true });
  await expect(failures).toBeVisible();
  await failures.getByRole('button', { name: 'Sync email', exact: true }).click();
  await expect(page.getByRole('status')).toContainText(['Sync finished.']);
});

test('sync button fits beside the logo in the narrow native macOS header', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('satchel.ui.sidebarWidth', '220'));
  await page.goto('/settings');
  const header = page.locator('.sidebar-brand');
  const button = header.locator('.sync-button');
  await expect(button).toBeVisible();
  await page.evaluate(() => {
    document.documentElement.dataset.runtimePlatform = 'macos';
    document.documentElement.dataset.windowChrome = 'true';
  });
  const brand = await header.boundingBox();
  const sync = await button.boundingBox();
  const name = await header.locator('h1').boundingBox();
  expect(sync!.x + sync!.width).toBeLessThanOrEqual(brand!.x + brand!.width);
  expect(name!.x + name!.width).toBeLessThanOrEqual(sync!.x);
  expect(await header.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
});

test('the phone list searches and filters above itself, not in the drawer', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  const filter = page.locator('.list-pane').getByRole('button', { name: 'Filter and view' });
  await filter.click();
  const menu = page.getByRole('menu', { name: 'Filter and view' });
  await menu.getByRole('menuitemcheckbox', { name: 'All accounts' }).click();
  await expect(page.locator('.pane-header')).toContainText('All accounts');

  const field = page.getByRole('searchbox', { name: 'Search this list' });
  await field.fill('Google');
  const results = page.getByRole('region', { name: 'Search results' });
  await results.getByText('Google AI Studio quota warning').click();
  await expect(page).toHaveURL(/\/thread\//);
  await page.goBack();
  await expect(field).toHaveValue('Google');
  await expect(results.getByText('Google AI Studio quota warning')).toBeVisible();

  await page.getByRole('button', { name: 'Menu', exact: true }).click();
  const drawer = page.locator('.mobile-sidebar-panel');
  await expect(drawer).toBeVisible();
  await expect(drawer.getByRole('searchbox')).toHaveCount(0);
  await expect(drawer.getByRole('button', { name: 'Settings' })).toBeVisible();
  await expect(drawer.getByRole('button', { name: 'Calendar' })).toBeVisible();
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
