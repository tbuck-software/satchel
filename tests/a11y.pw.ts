import { expect, test, type Page } from './mock-contract.fixture';

test('keeps contact actions in the mobile pane and saves a new contact', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/contacts');
  const add = page.getByRole('button', { name: 'Add contact', exact: true });
  await expect(add).toBeVisible();
  for (const control of [add, page.getByRole('button', { name: 'Address book settings', exact: true })]) {
    const box = await control.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  }
  await add.click();
  const dialog = page.getByRole('dialog', { name: 'Add contact', exact: true });
  await dialog.getByLabel('Email address').fill('mobile-contact@example.com');
  await dialog.getByLabel('Name (optional)', { exact: true }).fill('Mobile Contact');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(page.locator('.contact-list button').filter({ hasText: 'mobile-contact@example.com' })).toContainText('Mobile Contact');
});

async function openInbox(page: Page) {
  await page.goto('/');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
}

async function switchToListMode(page: Page) {
  await page.getByRole('button', { name: 'View', exact: true }).click();
  await page.getByRole('menuitemradio', { name: 'List', exact: true }).click();
}

/**
 * Elements inside a listbox that are neither an option nor inside one. A
 * listbox owns options only; anything else is flattened or skipped by screen
 * readers.
 */
function listboxStrays(page: Page) {
  return page.evaluate(() => Array.from(document.querySelectorAll('[role="listbox"]')).flatMap((box) =>
    Array.from(box.querySelectorAll('*'))
      .filter((node) => {
        const role = node.getAttribute('role');
        if (role === 'option' || role === 'presentation' || node.closest('[role="option"]')) return false;
        return role !== null || ['A', 'ARTICLE', 'BUTTON', 'H2', 'H3', 'H4', 'SECTION'].includes(node.tagName);
      })
      .map((node) => `${box.getAttribute('aria-label')}: ${node.tagName.toLowerCase()}${node.getAttribute('role') ? `[role=${node.getAttribute('role')}]` : ''}`)));
}

test.describe('smart view', () => {
  test.beforeEach(async ({ page }) => {
    await openInbox(page);
  });

  test('preserves contact rows layout and selected appearance through the UI kit', async ({ page }) => {
    await page.evaluate(() => localStorage.setItem('satchel.pinnedContacts', JSON.stringify(['sarah@app.io'])));
    await page.reload();
    const row = page.locator('aside button.card-button').filter({ has: page.locator('.contact-pin') }).first();
    await expect(row).toBeVisible();
    await expect(row).toHaveCSS('display', 'grid');
    await expect(row).toHaveCSS('padding-left', '10px');
    await row.click();
    await page.mouse.move(1000, 0);
    await expect(row).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  });

  test('keeps cards, headers and buttons out of every listbox', async ({ page }) => {
    const list = page.getByRole('group', { name: 'Message list' });
    await expect(list.getByRole('option').first()).toBeVisible();

    expect(await listboxStrays(page)).toEqual([]);
  });

  test('exposes the priority rows as their own listbox', async ({ page }) => {
    const priority = page
      .getByRole('group', { name: 'Message list' })
      .getByRole('listbox', { name: 'Priority' });

    await expect(priority).toHaveAttribute('aria-multiselectable', 'true');
    await expect(priority.getByRole('option').first()).toBeVisible();
    // The card header stays reachable as a button beside the listbox.
    await expect(page.getByRole('button', { name: /^Priority \d+$/ })).toBeVisible();
  });
});

/**
 * The thread list is where the app is used 90 % of the time. It used to expose
 * every row as `role="button" tabindex="0"`, which made tabbing past the list
 * impossible and left read/unread encoded purely in font weight.
 */
test.describe('list view', () => {
  test.beforeEach(async ({ page }) => {
    await openInbox(page);
    await switchToListMode(page);
  });

  test('exposes the thread rows as listboxes', async ({ page }) => {
    const list = page.getByRole('group', { name: 'Message list' });
    const firstBox = list.getByRole('listbox').first();
    await expect(firstBox).toBeVisible();
    await expect(firstBox).toHaveAttribute('aria-multiselectable', 'true');
    await expect(firstBox.getByRole('option').first()).toBeVisible();
    expect(await listboxStrays(page)).toEqual([]);
  });

  test('announces read state, sender and subject per row', async ({ page }) => {
    const firstRow = page.getByRole('group', { name: 'Message list' }).getByRole('option').first();
    const label = await firstRow.getAttribute('aria-label');

    expect(label).toBeTruthy();
    // Read state must be in the accessible name, not only in the styling.
    expect(label).toMatch(/^(Unread|Read), /);
  });

  test('keeps exactly one row in the tab order (roving tabindex)', async ({ page }) => {
    const list = page.getByRole('group', { name: 'Message list' });
    await expect(list.getByRole('option')).not.toHaveCount(0);

    const tabbable = list.locator('[role="option"][tabindex="0"]');
    await expect(tabbable).toHaveCount(1);
  });

  test('reports each row position against its whole listbox, not the rendered window', async ({ page }) => {
    const box = page.getByRole('group', { name: 'Message list' }).getByRole('listbox').first();
    const firstRow = box.getByRole('option').first();

    const posInSet = await firstRow.getAttribute('aria-posinset');
    const setSize = await firstRow.getAttribute('aria-setsize');
    const renderedCount = await box.getByRole('option').count();

    expect(Number(posInSet)).toBeGreaterThan(0);
    expect(Number(setSize)).toBeGreaterThanOrEqual(renderedCount);
  });

  test('marks selection with aria-selected rather than colour alone', async ({ page }) => {
    const firstRow = page.getByRole('group', { name: 'Message list' }).getByRole('option').first();
    await firstRow.click();
    await expect(firstRow).toHaveAttribute('aria-selected', 'true');
  });
});

/** The pane dividers were mouse-only separators: no tab stop, no keys, no value. */
test('resizes both panes from the keyboard within the store limits', async ({ page }) => {
  await openInbox(page);
  const sidebar = page.getByRole('separator', { name: 'Resize the sidebar' });
  const list = page.getByRole('separator', { name: 'Adjust the message list width' });

  await expect(sidebar).toHaveAttribute('aria-valuemin', '220');
  await expect(sidebar).toHaveAttribute('aria-valuemax', '420');
  await expect(list).toHaveAttribute('aria-valuemin', '320');
  await expect(list).toHaveAttribute('aria-valuemax', '760');

  await sidebar.focus();
  await page.keyboard.press('Home');
  await expect(sidebar).toHaveAttribute('aria-valuenow', '220');
  await page.keyboard.press('ArrowRight');
  await expect(sidebar).toHaveAttribute('aria-valuenow', '236');
  await expect(page.locator('aside').first()).toHaveJSProperty('offsetWidth', 236);

  await list.focus();
  const before = Number(await list.getAttribute('aria-valuenow'));
  await page.keyboard.press('Shift+ArrowLeft');
  await expect(list).toHaveAttribute('aria-valuenow', String(before - 64));
});

/**
 * Icon buttons are 26-38px. On a touch screen the hit area grows to 44px
 * around the icon without moving anything.
 */
test.describe('touch targets', () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test('lets a finger hit the calendar icon buttons 21px off centre', async ({ page }) => {
    await page.goto('/calendar');
    const buttons = ['Menu', 'Back (←)', 'Forward (→)'].map((name) => page.getByRole('button', { name, exact: true }));
    for (const button of buttons) await expect(button).toBeVisible();

    for (const button of buttons) {
      const box = await button.boundingBox();
      expect(box!.width).toBeLessThan(44);
      const hits = await button.evaluate((element) => {
        const rect = element.getBoundingClientRect();
        const x = rect.left + rect.width / 2;
        const y = rect.top + rect.height / 2;
        return [[0, -21], [0, 21], [-21, 0], [21, 0]].map(([dx, dy]) =>
          document.elementFromPoint(x + dx!, y + dy!)?.closest('.icon-button') === element);
      });
      expect(hits).toEqual([true, true, true, true]);
    }
  });
});
