import { fileURLToPath } from 'node:url';
import { defineConfig } from '@playwright/test';

const cspPort = process.env.SATCHEL_CSP_PORT ?? '4179';
const cspBaseUrl = `http://127.0.0.1:${cspPort}`;

export default defineConfig({
  testDir: '.',
  testMatch: ['csp.pw.ts'],
  fullyParallel: false,
  workers: 1,
  reporter: 'line',
  outputDir: '/tmp/satchel-playwright-csp-results',
  use: {
    baseURL: cspBaseUrl,
    browserName: 'chromium',
    viewport: { width: 1440, height: 900 },
  },
  // Serves the real production build, not the dev server. The build runs in
  // the `test:csp` script, not here: hidden in this command, a reused server
  // skipped it and the CSP was checked against whatever `dist/` held.
  webServer: {
    command: `pnpm exec vite preview --outDir dist --host 127.0.0.1 --port ${cspPort} --strictPort`,
    // Playwright resolves webServer.cwd against the config file, which lives in
    // tests/; vite needs the repo root.
    cwd: fileURLToPath(new URL('..', import.meta.url)),
    url: `${cspBaseUrl}/index.html`,
    // A stranger's server on the selected port once answered green for the real
    // one. Locally the port is reused for speed, but never in CI.
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
