import { describe, expect, it } from 'vitest';
import { createStartupEventBuffer } from './startupEventBuffer';

describe('startup event buffer', () => {
  it('holds early events until ready and preserves order across readiness', async () => {
    const buffer = createStartupEventBuffer();
    const seen: string[] = [];
    buffer.push(async () => { seen.push('notification'); });
    buffer.push(() => { seen.push('tray'); });
    expect(seen).toEqual([]);
    const flush = buffer.flush();
    buffer.push(() => { seen.push('auth'); });
    await flush;
    await buffer.flush();
    expect(seen).toEqual(['notification', 'tray', 'auth']);
  });

  it('drops buffered events when the app unmounts before readiness', async () => {
    const buffer = createStartupEventBuffer();
    const seen: string[] = [];
    buffer.push(() => { seen.push('stale'); });
    buffer.dispose();
    await buffer.flush();
    buffer.push(() => { seen.push('late'); });
    await buffer.flush();
    expect(seen).toEqual([]);
  });
});
