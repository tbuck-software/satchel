import { test as base, expect } from '@playwright/test';

export { expect };
export type { Page } from '@playwright/test';

export const test = base.extend<{ commandContract: void }>({
  commandContract: [async ({ page }, use) => {
    const violations: string[] = [];
    page.on('console', (message) => {
      if (message.text().includes('mock backend: invalid payload')) violations.push(message.text());
    });
    await use();
    expect(violations, 'mock calls must match Rust command arguments').toEqual([]);
  }, { auto: true }],
});
