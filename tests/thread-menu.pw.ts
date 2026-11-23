import { expect, test } from './mock-contract.fixture';

// German labels are the longest; "E-Mails vom Absender aufräumen …" once
// widened the menu's grid track past the menu and made it scroll sideways.
// macOS shortcut glyphs (⇧E), because that is where the app ships.
test('the thread toolbar menu fits its German labels without scrolling sideways', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(Navigator.prototype, 'platform', { configurable: true, get: () => 'MacIntel' });
    localStorage.setItem('satchel.onboardingComplete', 'true');
    localStorage.setItem('satchel.locale', 'de');
  });
  await page.goto('/');
  const trigger = page.getByTitle('Weitere Aktionen');
  await expect(trigger).toBeVisible({ timeout: 20_000 });
  await trigger.click();

  const menu = page.getByRole('menu', { name: 'Weitere Aktionen' });
  await expect(menu.getByRole('group', { name: 'Textgröße' })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /E-Mails vom Absender aufräumen/ })).toContainText('⇧');
  const geometry = await menu.evaluate((element) => {
    const bounds = element.getBoundingClientRect();
    const rows = Array.from(element.querySelectorAll<HTMLElement>('[role^="menuitem"], [role="group"]'));
    return {
      scrollWidth: element.scrollWidth,
      clientWidth: element.clientWidth,
      overflowing: rows
        .filter((node) => node.getBoundingClientRect().right > bounds.right + 0.5)
        .map((node) => node.getAttribute('aria-label') ?? node.textContent?.trim()),
      truncated: Array.from(element.querySelectorAll<HTMLElement>('.menu-item__label'))
        .filter((label) => label.scrollWidth > label.clientWidth)
        .map((label) => label.textContent?.trim()),
    };
  });
  expect(geometry.overflowing).toEqual([]);
  expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth);
  expect(geometry.truncated).toEqual([]);
});
