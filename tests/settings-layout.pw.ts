import { expect, test } from './mock-contract.fixture';

for (const width of [1280, 1440]) {
  test(`Labs fits the settings dialog at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await page.addInitScript(() => {
      localStorage.setItem('satchel.onboardingComplete', 'true');
      localStorage.setItem('satchel.locale', 'de');
    });
    await page.goto('/settings');
    const dialog = page.getByRole('dialog', { name: 'Einstellungen', exact: true });
    await expect(dialog).toBeVisible();
    await dialog.getByRole('navigation').getByRole('button', { name: /Labs/ }).click();
    await expect(dialog.locator('.labs-settings')).toBeVisible();
    const geometry = await dialog.evaluate((element) => {
      const selectors = ['.settings-dialog__main', '.settings-view', '.settings-view__scroll', '.settings-view__content', '.labs-settings'];
      return selectors.map((selector) => {
        const node = element.querySelector<HTMLElement>(selector)!;
        const bounds = node.getBoundingClientRect();
        return { selector, width: node.clientWidth, scrollWidth: node.scrollWidth,
          right: bounds.right, dialogRight: element.getBoundingClientRect().right };
      });
    });
    await page.screenshot({ path: `output/review/settings-overflow/labs-${width}.png` });
    expect(geometry.filter((row) => row.scrollWidth > row.width + 1 || row.right > row.dialogRight + 1)).toEqual([]);
  });
}

test('settings sections stay inside a narrow desktop dialog', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript(() => {
    localStorage.setItem('satchel.onboardingComplete', 'true');
    localStorage.setItem('satchel.locale', 'de');
  });
  await page.goto('/settings');
  const dialog = page.getByRole('dialog', { name: 'Einstellungen', exact: true });
  await expect(dialog).toBeVisible();
  const sections = dialog.getByRole('navigation').getByRole('listitem').getByRole('button');
  await expect(sections.first()).toBeVisible();
  const count = await sections.count();
  expect(count).toBeGreaterThan(10);
  const overflow: { section: string; excess: number }[] = [];
  const clipped: { section: string; element: string; container: string }[] = [];
  for (let index = 0; index < count; index += 1) {
    const section = sections.nth(index);
    const name = (await section.textContent())!.trim();
    await section.click();
    await page.waitForLoadState('networkidle');
    const excess = await dialog.locator('.settings-view__scroll').evaluate((node) => node.scrollWidth - node.clientWidth);
    if (excess > 1) overflow.push({ section: name, excess });
    const hidden = await dialog.locator('.settings-view__content').evaluate((content) => {
      const issues: { element: string; container: string }[] = [];
      for (const node of content.querySelectorAll<HTMLElement>('button, input, select, textarea, [role="switch"], .setting-label, .setting-hint')) {
        const rect = node.getBoundingClientRect();
        if (rect.width <= 2 || rect.height <= 2 || getComputedStyle(node).visibility !== 'visible') continue;
        for (let parent = node.parentElement; parent && content.contains(parent); parent = parent.parentElement) {
          const style = getComputedStyle(parent);
          // A local horizontal scroller keeps its contents reachable.
          if (['auto', 'scroll'].includes(style.overflowX) && parent.scrollWidth > parent.clientWidth + 1) break;
          if (!['hidden', 'clip'].includes(style.overflowX)) continue;
          const bounds = parent.getBoundingClientRect();
          if (rect.left < bounds.left - 1 || rect.right > bounds.right + 1) {
            issues.push({ element: node.getAttribute('aria-label') || node.textContent?.trim().slice(0, 90) || node.tagName,
              container: parent.className });
            break;
          }
        }
      }
      return issues;
    });
    clipped.push(...hidden.map((item) => ({ section: name, ...item })));
  }
  expect(overflow).toEqual([]);
  expect(clipped, 'controls and setting copy must remain inside clipping cards').toEqual([]);
});

test('Labs also fits the phone settings page', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(() => {
    localStorage.setItem('satchel.onboardingComplete', 'true');
    localStorage.setItem('satchel.locale', 'de');
  });
  await page.goto('/settings?section=labs');
  const labs = page.locator('.labs-settings');
  await expect(labs).toBeVisible();
  expect(await labs.evaluate((node) => node.scrollWidth - node.clientWidth)).toBeLessThanOrEqual(1);
  expect(await page.locator('.settings-view__scroll').evaluate((node) => node.scrollWidth - node.clientWidth)).toBeLessThanOrEqual(1);
});
