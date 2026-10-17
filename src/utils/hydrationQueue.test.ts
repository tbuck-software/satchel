import { describe, expect, it } from 'vitest';
import { createHydrationQueue } from './hydrationQueue';

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('hydration queue', () => {
  it('never runs more than three jobs at once', async () => {
    const pending = new Map<string, ReturnType<typeof deferred>>();
    let peak = 0;
    let active = 0;
    const queue = createHydrationQueue(async (key) => {
      active += 1;
      peak = Math.max(peak, active);
      const job = deferred();
      pending.set(key, job);
      await job.promise;
      active -= 1;
    });

    for (let index = 0; index < 20; index += 1) queue.request(`thread-${index}`);
    await flush();
    expect(pending.size).toBe(3);

    for (let round = 0; round < 20; round += 1) {
      for (const job of pending.values()) job.resolve();
      await flush();
    }

    expect(peak).toBe(3);
    expect(pending.size).toBe(20);
  });

  it('starts each key once, in the order it was asked for', async () => {
    const started: string[] = [];
    const queue = createHydrationQueue(async (key) => {
      started.push(key);
    }, 1);

    queue.request('a');
    queue.request('b');
    queue.request('a');
    await flush();
    await flush();

    expect(started).toEqual(['a', 'b']);
  });

  it('drops what is still waiting on reset', async () => {
    const started: string[] = [];
    const gate = deferred();
    const queue = createHydrationQueue(async (key) => {
      started.push(key);
      await gate.promise;
    }, 1);

    queue.request('old-topic-1');
    queue.request('old-topic-2');
    await flush();
    queue.reset();
    queue.request('new-topic-1');
    gate.resolve();
    await flush();
    await flush();

    expect(started).toEqual(['old-topic-1', 'new-topic-1']);
  });

  it('lets a failed job be asked for again', async () => {
    let attempts = 0;
    const queue = createHydrationQueue(async () => {
      attempts += 1;
      if (attempts === 1) throw new Error('offline');
    });

    queue.request('thread');
    await flush();
    queue.request('thread');
    await flush();

    expect(attempts).toBe(2);
  });
});
