// @vitest-environment node
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import { expect, it } from 'vitest';

it('keeps spoofing-table Unicode characters as data rather than export identifiers', async () => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'satchel-vite-json-'));
  const server = await createServer({
    cacheDir,
    configFile: fileURLToPath(new URL('../vite.config.ts', import.meta.url)),
    server: { host: '127.0.0.1', port: 0, hmr: false, ws: false, watch: null },
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  try {
    await server.listen();
    const address = server.httpServer?.address();
    if (!address || typeof address === 'string') throw new Error('Missing test server address');
    const base = `http://127.0.0.1:${address.port}`;
    const consumerUrl = `${base}/src/utils/senderSpoofing.ts`;
    const consumer = await (await fetch(consumerUrl)).text();
    const dependency = consumer.match(/from ["']([^"']*confusablesAscii\.json[^"']*)["']/)?.[1];
    expect(dependency).toBeDefined();
    const response = await fetch(new URL(dependency ?? '', consumerUrl));
    expect(response.ok).toBe(true);
    const code = await response.text();
    // Vite's Unicode tables can be newer than WebKit's identifier grammar.
    expect(code).not.toMatch(/^export (?:const|let|var) \P{ASCII}/mu);
    const module = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
    const expected = JSON.parse(readFileSync(new URL('../src/utils/data/confusablesAscii.json', import.meta.url), 'utf8'));
    const data = typeof module.default === 'string' ? JSON.parse(module.default) : module.default;
    expect(data).toEqual(expected);
  } finally {
    await server.close();
    rmSync(cacheDir, { recursive: true, force: true });
  }
}, 30_000);
