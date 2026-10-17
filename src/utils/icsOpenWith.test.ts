import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { drainPendingIcsPaths } from './icsOpenWith';

describe('open-with invitations buffered across a cold start', () => {
  it('imports every buffered path once, in the order it was launched with', async () => {
    const imported: string[] = [];
    const takePending = vi
      .fn<() => Promise<string[]>>()
      .mockResolvedValueOnce(['first.ics', 'second.ics'])
      .mockResolvedValueOnce([]);

    await drainPendingIcsPaths(takePending, async (path) => {
      imported.push(path);
    });
    await drainPendingIcsPaths(takePending, async (path) => {
      imported.push(path);
    });

    expect(imported).toEqual(['first.ics', 'second.ics']);
  });

  it('survives a shell that does not know the command', async () => {
    const warn = console.warn;
    console.warn = () => {};
    const importPath = vi.fn();
    try {
      await drainPendingIcsPaths(async () => {
        throw new Error('unknown command');
      }, importPath);
    } finally {
      console.warn = warn;
    }

    expect(importPath).not.toHaveBeenCalled();
  });

  /**
   * Draining empties the buffer, so anything arriving afterwards has to reach
   * the listener. Draining first would drop a path that lands in between.
   */
  it('is called after the listener is registered, not before', () => {
    const source = readFileSync(new URL('../bootstrap.ts', import.meta.url), 'utf8');
    const listener = source.indexOf("mod.listen('ics:open-with'");
    const drain = source.indexOf('drainPendingIcsPaths(backendTakePendingOpenWith');

    expect(listener).toBeGreaterThan(-1);
    expect(drain).toBeGreaterThan(listener);
  });
});
