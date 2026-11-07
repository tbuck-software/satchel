import { expect, test } from './mock-contract.fixture';

async function openNewEvent(page: import('@playwright/test').Page) {
  await page.goto('/calendar');
  // The calendar loads lazily; the button appears once it has.
  await page.getByRole('button', { name: 'New event', exact: true }).first().click();
  const editor = page.getByRole('dialog', { name: 'New event' });
  await expect(editor).toBeVisible();
  await editor.getByPlaceholder('person@example.com, team@example.com').fill('lena.vogel@app.io, marco@studio-rossi.example');
  return editor;
}

test('the event editor shows who is busy and moves the slot to the next common free time', async ({ page }) => {
  const editor = await openNewEvent(page);
  const strip = editor.getByTestId('availability-strip');
  // You, Lena (inside the organisation) and Marco (outside: unknown).
  await expect(strip.locator('.strip__track')).toHaveCount(3);
  await expect(strip.locator('.strip__track--unknown')).toHaveCount(1);
  await expect(strip.getByText('Striped rows:')).toBeVisible();

  const start = editor.locator('input[type="datetime-local"]').first();
  const end = editor.locator('input[type="datetime-local"]').nth(1);
  const before = await start.inputValue();
  await strip.getByRole('button', { name: 'Next time everyone is free' }).click();
  await expect(start).not.toHaveValue(before);
  await expect(strip.getByText('Not counted, their availability is unknown: marco@studio-rossi.example')).toBeVisible();
  const moved = new Date(await start.inputValue()).getTime();
  expect(new Date(await end.inputValue()).getTime() - moved).toBe(60 * 60_000);

  // Keyboard: the day steps are buttons in the tab order.
  await strip.getByRole('button', { name: 'Next day' }).focus();
  await page.keyboard.press('Enter');
  await expect.poll(async () => new Date(await start.inputValue()).getTime()).not.toBe(moved);
  const stepped = new Date(await start.inputValue());
  const expected = new Date(moved);
  expected.setDate(expected.getDate() + 1);
  expect(stepped.getTime()).toBe(expected.getTime());
});

test('on a phone the editor says the same as a short list', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const editor = await openNewEvent(page);
  const strip = editor.getByTestId('availability-strip');
  await expect(strip.locator('.strip__list-row')).toHaveCount(3);
  await expect(strip.locator('.strip__list-row--unknown')).toContainText('Availability unknown');
  const width = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(width).toBeLessThanOrEqual(390);
});

test('inserts my availability into the composer as text', async ({ page }) => {
  await page.goto('/compose');
  await page.getByRole('button', { name: 'Insert my availability', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Insert my availability' });
  const preview = dialog.getByTestId('insert-availability-preview');
  await expect(preview).toContainText("I'm free for 30 minutes at these times (");
  await dialog.getByRole('button', { name: '1 h', exact: true }).click();
  await expect(preview).toContainText("I'm free for 1 hour at these times (");
  await dialog.getByRole('button', { name: 'Insert', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  const editor = page.locator('.rt-editor .ProseMirror');
  await expect(editor).toContainText("I'm free for 1 hour at these times (");
  await expect(editor.locator('ul li').first()).toBeVisible();
});
