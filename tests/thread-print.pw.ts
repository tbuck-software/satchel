import { expect, test, type Page } from './mock-contract.fixture';

interface PrintProbe {
  openCalls: number;
  printCalls: number;
  snapshots: string[];
}

declare global {
  interface Window {
    __satchelPrintProbe: PrintProbe;
  }
}

const firstMessage =
  'Users cannot sign in on staging since this morning. Steps: open staging login, enter valid credentials, click sign in, receive HTTP 500.';
const latestMessage =
  'Der OAuth-Fix ist auf Staging. Kannst du den Login noch einmal gegenprüfen?';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const probe: PrintProbe = {
      openCalls: 0,
      printCalls: 0,
      snapshots: [],
    };

    Object.defineProperty(window, '__satchelPrintProbe', {
      configurable: true,
      value: probe,
    });
    Object.defineProperty(window, 'open', {
      configurable: true,
      value: () => {
        probe.openCalls += 1;
        return null;
      },
    });
    Object.defineProperty(window, 'print', {
      configurable: true,
      value: () => {
        probe.printCalls += 1;
        probe.snapshots.push(
          document.querySelector('[data-thread-print]')?.textContent ?? '',
        );
      },
    });
  });

  await page.goto('/');
  await expect(page.getByTitle('More actions')).toBeVisible();
});

async function choosePrintScope(
  page: Page,
  label: 'Latest email only' | 'Whole conversation',
) {
  await page.getByTitle('More actions').click();
  await page.getByRole('menuitem', { name: 'Print', exact: true }).hover();
  await page.getByRole('menuitem', { name: label, exact: true }).click();
}

test('prints the latest message in the current window when popups are unavailable', async ({ page }) => {
  await choosePrintScope(page, 'Latest email only');

  const probe = await page.evaluate(() => window.__satchelPrintProbe);
  expect(probe.openCalls).toBe(0);
  expect(probe.printCalls).toBe(1);
  expect(probe.snapshots).toHaveLength(1);
  expect(probe.snapshots[0]).toContain(latestMessage);
  expect(probe.snapshots[0]).not.toContain(firstMessage);
  await expect(page.getByText('Printing could not be started.')).toHaveCount(0);
});

test('prints every message in conversation order', async ({ page }) => {
  await choosePrintScope(page, 'Whole conversation');

  const probe = await page.evaluate(() => window.__satchelPrintProbe);
  expect(probe.openCalls).toBe(0);
  expect(probe.printCalls).toBe(1);
  expect(probe.snapshots).toHaveLength(1);
  expect(probe.snapshots[0]).toContain(firstMessage);
  expect(probe.snapshots[0]).toContain(latestMessage);
  expect(probe.snapshots[0].indexOf(firstMessage)).toBeLessThan(
    probe.snapshots[0].indexOf(latestMessage),
  );
});
