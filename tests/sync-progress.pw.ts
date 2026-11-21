import { expect, test } from '@playwright/test';

test('shows durable import progress, pauses offline and finishes without a permanent spinner', async ({ page }) => {
  await page.goto('/?mockImportPhase=folder&mockOutbox=0');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
  const status = page.locator('aside .sync-progress');
  await expect(status).toContainText('Loading older mail');
  await expect(status).toContainText('50%');
  await expect(status.getByRole('button')).toHaveAttribute('title', /20,000 of 40,000 messages/);
  // One line and a hairline, not a card that pushes the agenda down.
  expect((await status.boundingBox())!.height).toBeLessThanOrEqual(40);
  await expect(status.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '20000');
  const sync = page.locator('.sync-button');
  await expect(sync).toBeEnabled();
  await expect(sync.locator('.sync-button__icon--spinning')).toHaveCount(0);
  // Focus explicitly: WebKit does not focus buttons on pointer activation.
  await status.getByRole('button').focus();
  await status.getByRole('button').click();
  const details = page.getByRole('dialog', { name: 'Mail sync', exact: true });
  await expect(details).toBeVisible();
  await expect(details).toContainText('Archive · folder 4 of 12');
  await expect(details.getByRole('button', { name: 'Close', exact: true })).toBeFocused();
  await page.evaluate(async () => {
    const path = '/src/stores/connection.ts';
    const { useConnectionStore } = await import(path);
    const connection = useConnectionStore();
    connection.syncProgress = connection.syncProgress.map((row: { backfill_remaining: number }) => ({ ...row, backfill_remaining: 10000 }));
    connection.online = false;
  });
  await expect(details).toContainText('30,000 of 40,000 messages');
  await expect(details).toContainText('Paused while offline');
  await expect(status).toContainText('Paused while offline');
  await page.evaluate(async () => {
    const path = '/src/stores/connection.ts';
    const { useConnectionStore } = await import(path);
    const connection = useConnectionStore();
    connection.online = true;
    connection.syncing = false;
    connection.syncProgress = connection.syncProgress.map((row: { phase: string }) => ({ ...row, phase: 'waiting' }));
  });
  await expect(status).toContainText('Loading older mail');
  await expect(details).toContainText('Working in the background');
  await page.evaluate(async () => {
    const path = '/src/stores/connection.ts';
    const { useConnectionStore } = await import(path);
    useConnectionStore().syncProgress = [];
  });
  await expect(details).toContainText('Everything is up to date.');
  await expect(status).toHaveCount(0);
  await details.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(details).toHaveCount(0);
  await expect(sync).toBeFocused();
});

test('keeps phone import progress beside the account and its details open through completion', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?mockImportPhase=waiting&mockOutbox=0');
  await expect(page.locator('.large-title')).toBeVisible();
  const chip = page.locator('.large-title__meta .shell-status').getByRole('button');
  await expect(chip).toHaveText('Older mail 50%');
  await expect(page.locator('.mobile-sync-status')).toHaveCount(0);
  await chip.click();
  const details = page.getByRole('dialog', { name: 'Mail sync', exact: true });
  await expect(details).toContainText('20,000 of 40,000 messages');
  await page.evaluate(async () => {
    const connectionPath = '/src/stores/connection.ts';
    const { useConnectionStore } = await import(connectionPath);
    useConnectionStore().syncProgress = [];
  });
  await expect(details).toContainText('Everything is up to date.');
  await details.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(details).toHaveCount(0);
  await expect(page.locator('.large-title__switch')).toBeFocused();
});
