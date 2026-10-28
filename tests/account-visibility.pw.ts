import { mkdir } from 'node:fs/promises';
import { expect, test } from './mock-contract.fixture';

test.use({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, timezoneId: 'Europe/Berlin' });

for (const theme of ['dark', 'light']) {
  test(`resting account, direct access, peek and schedule settings in ${theme}`, async ({ page }, testInfo) => {
    await page.clock.setFixedTime(new Date('2026-10-25T11:00:00Z'));
    await page.addInitScript(theme => localStorage.setItem('satchel.theme', theme), theme);
    await page.goto('/?mockVisibility=1');
    const accountArea = page.getByRole('navigation', { name: 'Accounts', exact: true });
    await expect(accountArea.getByText('Resting', { exact: true })).toBeVisible();
    await expect(accountArea.getByRole('button', { name: 'Show today', exact: true })).toBeVisible();
    await expect(page.getByText('Sunday lunch at our place', { exact: true }).first()).toBeVisible();
    await expect(page.getByText('Staging login 500 after deployment', { exact: true })).toHaveCount(0);
    await expect(page.frameLocator('iframe').first().getByText('Hi Tom,', { exact: true })).toBeVisible();
    const artifactDir = process.env.SATCHEL_VISIBILITY_SCREENSHOTS;
    const screenshot = async (name: string) => {
      if (!artifactDir) return;
      await mkdir(artifactDir, { recursive: true });
      await page.screenshot({ path: `${artifactDir}/${name}-${theme}-${testInfo.project.name}.png` });
    };
    const inboxBadge = page.locator('aside').first().getByRole('button', { name: /^Inbox/ }).locator('.nav-item__count');
    // Opening the first personal message marks it read after hydration:
    // two left in Personal, two in the rowing club.
    await expect(inboxBadge).toHaveText('4');
    // Each account carries its own share of that number, the resting one too.
    const accountCount = (name: string | RegExp) => accountArea.getByRole('button', { name }).locator('.nav-item__count');
    await expect(accountCount(/^All accounts/)).toHaveText('4');
    await expect(accountCount(/^Personal/)).toHaveText('2');
    await expect(accountCount(/^Rowing club/)).toHaveText('2');
    await expect(accountCount(/^Work/)).toHaveText(/^[1-9]\d*$/);
    // Right-click recolours an account in place.
    const personal = accountArea.getByRole('button', { name: /^Personal/ });
    await personal.click({ button: 'right' });
    const accountMenu = page.getByRole('menu', { name: 'Personal', exact: true });
    await accountMenu.getByRole('menuitemradio', { name: 'Purple', exact: true }).click();
    await expect(accountMenu).toHaveCount(0);
    await expect(personal.locator('.account-avatar')).toHaveAttribute('style', /--account-color: #a97fff/);
    const restingCount = Number(await inboxBadge.textContent());
    await screenshot('inbox-resting');
    await accountArea.getByRole('button', { name: 'Show today', exact: true }).hover();
    await screenshot('sidebar-show-today');
    // Direct access does not wake the account or change the policy.
    await accountArea.getByTitle('Open tom@satchel.dev, including resting mail').click();
    await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
    await expect(accountArea.getByText('Resting', { exact: true })).toBeVisible();
    await accountArea.getByRole('button', { name: /^All accounts/ }).click();
    await accountArea.getByRole('button', { name: 'Show today', exact: true }).click();
    await expect(accountArea.getByText('Resting', { exact: true })).toHaveCount(0);
    await expect.poll(async () => Number(await inboxBadge.textContent())).toBeGreaterThan(restingCount);
    await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
    await page.keyboard.press('ControlOrMeta+k');
    const palette = page.getByRole('dialog', { name: 'Search and commands' });
    await palette.getByRole('combobox').fill('Hide Work');
    await palette.getByRole('option', { name: /Hide Work/ }).click();
    await expect(accountArea.getByText('Resting', { exact: true })).toBeVisible();
    await expect(page.getByText('Staging login 500 after deployment', { exact: true })).toHaveCount(0);
    await expect(inboxBadge).toHaveText(String(restingCount));
    // Route inside the same app to retain the two-account fixture.
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('button', { name: 'Accounts', exact: true }).click();
    const work = page.locator('.account-card').filter({ hasText: 'tom@satchel.dev' });
    await expect(work.getByRole('button', { name: 'On a schedule', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await expect(work.locator('input[type="time"]').first()).toHaveValue('09:00');
    await screenshot('account-schedule');
    await work.getByLabel('From, optional', { exact: true }).fill('10:00');
    await work.getByLabel('To, optional', { exact: true }).fill('18:00');
    await work.getByRole('button', { name: 'Wednesday', exact: true }).click();
    await work.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(work.getByRole('button', { name: 'Save', exact: true })).toHaveCount(0);
    await work.getByRole('button', { name: 'Only when I open it', exact: true }).click();
    await work.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(work.getByRole('button', { name: 'Save', exact: true })).toHaveCount(0);
    // A new local day expires a peek; a schedule boundary updates without reload.
    await work.getByRole('button', { name: 'On a schedule', exact: true }).click();
    await work.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(work.getByLabel('From, optional', { exact: true })).toHaveValue('10:00');
    await expect(work.getByRole('button', { name: 'Wednesday', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await page.clock.setFixedTime(new Date('2026-10-26T09:00:00Z'));
    await page.clock.runFor(15_100);
    await expect(work.getByText('Visible', { exact: true })).toBeVisible();
  });
}
