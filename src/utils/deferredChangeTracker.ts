import { getCurrentScope, onScopeDispose, ReactiveEffect } from 'vue';

export interface DeferredChangeTracker<T> {
  /** Compares now if anything the source reads has changed since the last look. */
  settle(): void;
  /** Takes the current value as the new reference without reporting a change. */
  rebaseline(): void;
  /** The value from the last comparison. */
  current(): T;
  stop(): void;
}

/**
 * Notices that a derived value may have changed without computing it.
 *
 * A `watch` on an expensive getter runs that getter on every change of its
 * dependencies — in the composer that meant serialising the whole draft per
 * keystroke. Here a dependency change only (re)starts a timer; the getter runs
 * once the changes pause, or earlier when a caller needs the answer now and
 * calls `settle()`. `onChange` fires only when the value actually differs.
 */
export function deferredChangeTracker<T>(
  source: () => T,
  onChange: (next: T) => void,
  delayMs: number,
): DeferredChangeTracker<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  // With a scheduler, a dependency change calls it instead of re-running the
  // source; `run()` evaluates and collects the dependencies afresh.
  const tracker = new ReactiveEffect(source);
  tracker.scheduler = () => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(settle, delayMs);
  };
  let last = tracker.run();

  function cancelTimer() {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  }

  function settle() {
    if (timer === null) return;
    cancelTimer();
    const next = tracker.run();
    if (Object.is(next, last)) return;
    last = next;
    onChange(next);
  }

  function rebaseline() {
    cancelTimer();
    last = tracker.run();
  }

  function stop() {
    cancelTimer();
    tracker.stop();
  }

  if (getCurrentScope()) onScopeDispose(stop);

  return { settle, rebaseline, current: () => last, stop };
}
