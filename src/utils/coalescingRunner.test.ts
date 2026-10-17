import { describe, expect, it, vi } from 'vitest';
import { createCoalescingRunner } from './coalescingRunner';

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe('createCoalescingRunner', () => {
  it('shares one active task and runs one follow-up for work queued while it is pending', async () => {
    const first = deferred();
    let revision = 0;
    let savedRevision = -1;
    let calls = 0;
    let active = 0;
    let maxActive = 0;
    const task = vi.fn(async () => {
      const revisionAtStart = revision;
      calls += 1;
      active += 1;
      maxActive = Math.max(maxActive, active);
      if (calls === 1) await first.promise;
      savedRevision = revisionAtStart;
      active -= 1;
      return calls;
    });
    const runner = createCoalescingRunner(task, () => savedRevision !== revision);

    const initial = runner.run();
    revision += 1;
    const queued = runner.run();
    const duplicate = runner.run();

    expect(task).toHaveBeenCalledTimes(1);
    expect(queued).toBe(initial);
    expect(duplicate).toBe(initial);

    first.resolve();

    await expect(initial).resolves.toBe(2);
    expect(task).toHaveBeenCalledTimes(2);
    expect(maxActive).toBe(1);
  });

  it('does not repeat when queued callers have no newer work', async () => {
    const first = deferred();
    let dirty = false;
    const task = vi.fn(async () => {
      await first.promise;
      return 'saved';
    });
    const runner = createCoalescingRunner(task, () => dirty);

    const initial = runner.run();
    runner.run();
    first.resolve();

    await expect(initial).resolves.toBe('saved');
    expect(task).toHaveBeenCalledTimes(1);
  });

  it('cancels queued work and exposes the active task for terminal actions', async () => {
    const first = deferred();
    let calls = 0;
    const task = vi.fn(async () => {
      calls += 1;
      await first.promise;
      return calls;
    });
    const runner = createCoalescingRunner(task, () => true);

    const active = runner.run();
    runner.run();
    expect(runner.settle()).toBe(active);

    runner.cancelPending();
    first.resolve();

    await expect(active).resolves.toBe(1);
    expect(task).toHaveBeenCalledTimes(1);
    expect(runner.settle()).toBeNull();
  });
});
