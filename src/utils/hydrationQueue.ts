/**
 * Runs keyed jobs a few at a time, each key at most once.
 *
 * The topic feed started a full load for every linked thread at once — twenty
 * threads meant twenty loads racing for a message cache that keeps three. The
 * feed now asks for a thread when its card comes into view, and this decides
 * how many actually run.
 */
export interface HydrationQueue {
  /** Ask for `key`. Ignored when it already ran, runs, or waits. */
  request(key: string): void;
  /** Forget everything still waiting, and what already ran. Running jobs finish. */
  reset(): void;
  /** Jobs running right now. */
  readonly running: number;
}

export function createHydrationQueue(
  run: (key: string) => Promise<unknown>,
  concurrency = 3,
): HydrationQueue {
  const waiting: string[] = [];
  const seen = new Set<string>();
  let running = 0;
  let generation = 0;

  function pump() {
    while (running < concurrency && waiting.length > 0) {
      const key = waiting.shift()!;
      const startedIn = generation;
      running += 1;
      void Promise.resolve()
        .then(() => run(key))
        .catch(() => {
          // A failed hydration may be asked for again once it is back in view.
          if (startedIn === generation) seen.delete(key);
        })
        .finally(() => {
          running -= 1;
          pump();
        });
    }
  }

  return {
    request(key) {
      if (!key || seen.has(key)) return;
      seen.add(key);
      waiting.push(key);
      pump();
    },
    reset() {
      generation += 1;
      waiting.length = 0;
      seen.clear();
    },
    get running() {
      return running;
    },
  };
}
