import { expect, test } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';

test.use({ deviceScaleFactor: 2 });

for (const theme of ['dark', 'light'] as const) {
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    for (const surface of ['inbox', 'failures', 'reply-toast', 'maximised'] as const) {
      if (surface === 'maximised' && viewport.width < 640) continue;
      test(`queue failure ${surface}, ${theme}, ${viewport.width}`, async ({ page }, testInfo) => {
        await page.setViewportSize(viewport);
        await page.addInitScript((value) => localStorage.setItem('satchel.theme', value), theme);
        await page.goto('/');
        await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
        await expect(page.locator('.stack')).toBeAttached({ timeout: 30_000 });
        await expect(page.locator('.toast')).toHaveCount(0, { timeout: 20_000 });
        const indicator = viewport.width < 640
          // The phone shows a banner that names the count, with Review as its way in.
          ? page.getByRole('status').filter({ hasText: '2 items need attention' })
            .getByRole('button', { name: 'Review', exact: true }).filter({ visible: true })
          : page.locator('.sync-button');
        await expect(indicator).toBeVisible();
        await expect(indicator).toHaveAttribute('aria-haspopup', 'dialog');

        if (surface === 'failures') {
          await indicator.click();
          const failures = page.getByRole('dialog', { name: 'Needs attention', exact: true });
          await expect(failures).toBeVisible();
          await expect(page.locator('.sheet-root')).toHaveCSS('opacity', '1');
          await expect(failures).toContainText('too large');
          await expect(failures).toContainText('folder');
          await expect(failures.getByRole('button', { name: 'Try again', exact: true })).toBeVisible();
          await expect(failures.getByRole('button', { name: 'Outbox', exact: true })).toBeVisible();
          await page.keyboard.press('Escape');
          await expect(failures).toHaveCount(0);
          await expect(indicator).toBeFocused();
          await indicator.click();
          await expect(page.locator('.sheet-root')).toHaveCSS('opacity', '1');
        }

        if (surface === 'reply-toast' || surface === 'maximised') {
          if (viewport.width < 640) {
            await page.getByText('Staging login 500 after deployment', { exact: true }).first().click();
          }
          await page.keyboard.press('r');
          const composer = page.getByRole('dialog', { name: 'Compose email', exact: true });
          await expect(composer).toBeVisible({ timeout: 30_000 });
          await expect(composer.locator('[data-composer-history]')).toBeVisible();
          // Drive the real watcher. Keep the synthetic receipt alive during
          // geometry checks; its seven-second expiry was checked above.
          await page.evaluate(async () => {
            const storePath = '/src/stores/outbox.ts';
            const { useOutboxStore } = await import(storePath) as {
              useOutboxStore: () => { items: { id: string; status: string; action_type: string }[] };
            };
            const outbox = useOutboxStore();
            const failure = outbox.items.find((item) => item.status === 'failed' && item.action_type === 'send_message');
            if (!failure) throw new Error('missing failed send fixture');
            outbox.items = [...outbox.items, { ...failure, id: 'fresh-failure' }];
          });
          await expect(page.locator('.toast')).toHaveCount(1);
          await page.evaluate(async () => {
            const modulePath = '/src/composables/useToast.ts';
            const { useToast } = await import(modulePath) as { useToast: () => { state: { toasts: { expiresAt: number }[] } } };
            for (const item of useToast().state.toasts) item.expiresAt = Date.now() + 60_000;
          });
          await expect(page.locator('.toast')).toHaveCSS('opacity', '1');
          const overlaps = async () => {
            const a = (await composer.boundingBox())!;
            const b = (await page.locator('.toast').boundingBox())!;
            return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
          };
          await expect.poll(overlaps).toBe(false);
          if (surface === 'maximised') {
            await composer.getByRole('button', { name: 'Maximise composer' }).click();
            await expect.poll(async () => (await page.locator('.toast').boundingBox())!.y
              + (await page.locator('.toast').boundingBox())!.height <= (await composer.boundingBox())!.y).toBe(true);

          }
          for (const bar of await page.locator('.reply-bar').all()) {
            const a = (await bar.boundingBox())!;
            const b = (await page.locator('.toast').boundingBox())!;
            expect(a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height).toBe(false);
          }
        }

        const directory = process.env.SATCHEL_FAILURE_SCREENSHOT_DIR ?? testInfo.outputPath('screenshots');
        await mkdir(directory, { recursive: true });
        // WPE WebKit can stop animation frames after a capture. Each scenario
        // exports its image last, after all visibility and geometry assertions.
        await page.screenshot({ path: path.join(directory,
          `${testInfo.project.name}-${theme}-${viewport.width}x${viewport.height}@2x-${surface}.png`) });
      });
    }
  }
}
