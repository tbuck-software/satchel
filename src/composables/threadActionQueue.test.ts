import { describe, expect, it } from 'vitest';
import { createThreadActionQueue } from './threadActionQueue';

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(done => { resolve = done; });
  return { promise, resolve };
}

describe('thread action queue', () => {
  it('reserves overlapping batches in FIFO order while unrelated threads progress', async () => {
    const queue = createThreadActionQueue();
    const first = deferred();
    const order: string[] = [];
    const a = queue.run(['a', 'b'], async () => { order.push('first'); await first.promise; });
    const b = queue.run(['b', 'c'], async () => { order.push('second'); });
    const c = queue.run(['c'], async () => { order.push('third'); });
    await queue.run(['d'], async () => { order.push('independent'); });
    expect(order).toEqual(['first', 'independent']);
    first.resolve();
    await Promise.all([a, b, c]);
    expect(order).toEqual(['first', 'independent', 'second', 'third']);
  });

  it('continues a lane after an earlier action rejects', async () => {
    const queue = createThreadActionQueue();
    const first = queue.run(['a'], async () => { throw new Error('offline'); });
    const second = queue.run(['a'], async () => 'accepted');
    await expect(first).rejects.toThrow('offline');
    await expect(second).resolves.toBe('accepted');
  });
});
