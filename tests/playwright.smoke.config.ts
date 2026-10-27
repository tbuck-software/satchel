import { defineConfig } from '@playwright/test';

const smokePort = process.env.SATCHEL_SMOKE_PORT ?? '4178';
const smokeBaseUrl = `http://127.0.0.1:${smokePort}`;

export default defineConfig({
  testDir: '.',
  testMatch: ['account-visibility.pw.ts', 'composer.pw.ts', 'core-flows.pw.ts', 'email-rendering.pw.ts', 'mail-keyboard.pw.ts', 'thread-print.pw.ts', 'a11y.pw.ts', 'sheet-interactions.pw.ts', 'responsive-shell.pw.ts', 'responsive-search.pw.ts', 'failure-visibility.pw.ts'],
  fullyParallel: false,
  workers: 1,
  reporter: 'line',
  outputDir: '/tmp/satchel-playwright-smoke-results',
  /*
   * Playwright considers Vite ready after the HTML endpoint responds. On a
   * cold cache Vite still has to transform the main client module graph, which
   * can take longer than Playwright's 30-second default on external volumes.
   */
  timeout: 120_000,
  /*
   * The mock dev server occasionally misses the first navigation after a cold
   * start, which shows up as a beforeEach timeout rather than an assertion
   * failure. One retry keeps that flake from blocking local installs.
   */
  retries: 1,
  use: {
    baseURL: smokeBaseUrl,
    browserName: 'chromium',
    /*
     * The suite asserts against English copy. `en` is the default locale, but
     * pinning the browser language keeps the run independent of whoever's
     * machine it starts on.
     */
    locale: 'en-US',
    viewport: { width: 1440, height: 900 },
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'chromium' },
    /*
     * The mail renderer additionally runs in WebKit, because that is the engine
     * the app ships on: Tauri uses WKWebView on macOS. The two disagree exactly
     * where this renderer lives — WebKit runs no event listener registered on
     * the document of a frame sandboxed without `allow-scripts`. Mail links were
     * dead in every build from 2026-07-17 on while this suite stayed green in
     * Chromium alone.
     */
    {
      name: 'webkit-mail',
      testMatch: ['account-visibility.pw.ts', 'composer.pw.ts', 'email-rendering.pw.ts', 'mail-keyboard.pw.ts', 'sheet-interactions.pw.ts', 'failure-visibility.pw.ts'],
      use: { browserName: 'webkit' },
    },
  ],
  webServer: {
    command: `pnpm run dev:mock --host 127.0.0.1 --port ${smokePort}`,
    url: smokeBaseUrl,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
