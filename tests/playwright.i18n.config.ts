import { fileURLToPath } from 'node:url';
import { defineConfig } from '@playwright/test';

/*
 * The raw-key crawl against a mock production build: chunks precompiled and
 * hashed as they ship, the mock transport in place of Tauri. Build it first:
 *
 *   VITE_SATCHEL_MOCK=1 pnpm exec vite build --outDir dist-mock
 *
 * `pnpm run test:i18n` does both.
 */
const port = process.env.SATCHEL_I18N_PORT ?? '4180';

export default defineConfig({
  testDir: '.',
  testMatch: ['i18n-chunks.pw.ts'],
  fullyParallel: false,
  workers: 1,
  reporter: 'line',
  timeout: 120_000,
  // No retries: a missing key that shows up only sometimes is a race worth seeing.
  retries: 0,
  outputDir: process.env.SATCHEL_I18N_OUTPUT_DIR ?? fileURLToPath(new URL('../output/playwright/i18n', import.meta.url)),
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    browserName: 'chromium',
    locale: 'en-US',
    viewport: { width: 1440, height: 900 },
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium' }, { name: 'webkit', use: { browserName: 'webkit' } }],
  webServer: {
    // `vite preview` serves index.html for every route, like the app shell.
    command: `pnpm exec vite preview --outDir dist-mock --host 127.0.0.1 --port ${port} --strictPort`,
    cwd: fileURLToPath(new URL('..', import.meta.url)),
    url: `http://127.0.0.1:${port}/index.html`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
