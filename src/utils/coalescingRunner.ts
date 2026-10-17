export interface CoalescingRunner<T> {
  run: () => Promise<T>;
  cancelPending: () => void;
  settle: () => Promise<T> | null;
}

/**
 * Shares an active task between callers and collapses all calls received
 * while it is pending into at most one follow-up run.
 */
export function createCoalescingRunner<T>(
  task: () => Promise<T>,
  shouldRunAgain: () => boolean,
): CoalescingRunner<T> {
  let active: Promise<T> | null = null;
  let queued = false;

  function run(): Promise<T> {
    if (active) {
      queued = true;
      return active;
    }

    const cycle = (async () => {
      let result!: T;
      do {
        queued = false;
        result = await task();
      } while (queued && shouldRunAgain());
      return result;
    })();

    active = cycle.finally(() => {
      active = null;
      queued = false;
    });
    return active;
  }

  return {
    run,
    cancelPending: () => {
      queued = false;
    },
    settle: () => active,
  };
}
