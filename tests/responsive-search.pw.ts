import { expect, test } from './mock-contract.fixture';

test('tablet search pages beyond fifty results, retries a failed page and preserves scroll on Back', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 844 });
  await page.goto('/search?q=subject:Status&global=1&mockExtraThreads=137');
  await page.getByRole('heading', { name: 'Search', exact: true }).waitFor();
  const rows = page.locator('.search-scroll .result-row');
  const query = page.getByRole('searchbox');
  const more = page.getByRole('button', { name: 'Load more results', exact: true });
  await expect(rows).toHaveCount(50);
  await expect(page.locator('.result-count').first()).toHaveText('50 results loaded');

  await page.evaluate(() => {
    const url = new URL(location.href);
    url.searchParams.set('backendFailure', 'search_message_hits');
    history.replaceState(history.state, '', url);
  });
  await more.click();
  await expect(page.locator('.search-scroll [role="alert"]')).toBeVisible();
  await expect(rows).toHaveCount(50);
  await expect(more).toBeEnabled();

  await page.evaluate(() => {
    const url = new URL(location.href);
    url.searchParams.delete('backendFailure');
    history.replaceState(history.state, '', url);
  });
  await more.click();
  await expect(rows).toHaveCount(100);
  await expect(page.locator('.search-scroll [role="alert"]')).toHaveCount(0);
  await more.click();
  await expect(rows).toHaveCount(137);
  await expect(more).toHaveCount(0);
  await expect(page.locator('.result-count').first()).toHaveText('137 results');

  const hit = rows.nth(90);
  await hit.scrollIntoViewIfNeeded();
  const scroll = await page.locator('.search-scroll').evaluate((element) => element.scrollTop);
  expect(scroll).toBeGreaterThan(0);
  await hit.click();
  await expect(query).not.toBeVisible();
  await page.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(query).toBeVisible();
  await expect(query).toHaveValue('subject:Status');
  await expect(rows).toHaveCount(137);
  expect(await page.locator('.search-scroll').evaluate((element) => element.scrollTop)).toBeCloseTo(scroll, 0);
  await hit.click();
  await expect(query).not.toBeVisible();
  await page.goBack();
  await expect(query).toBeVisible();
  await expect(rows).toHaveCount(137);
  expect(await page.locator('.search-scroll').evaluate((element) => element.scrollTop)).toBeCloseTo(scroll, 0);
  await expect(page).toHaveURL(/global=1/);
});
