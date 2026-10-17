/** Accept events before stores load; replay them in arrival order after readiness. */
export function createStartupEventBuffer() {
  let ready = false;
  let disposed = false;
  const pending: Array<() => unknown | Promise<unknown>> = [];
  let tail: Promise<void> = Promise.resolve();

  function enqueue(task: () => unknown | Promise<unknown>) {
    tail = tail.then(async () => {
      if (!disposed) await task();
    }).catch((error) => console.warn('startup event handler failed', error));
  }

  return {
    push(task: () => unknown | Promise<unknown>) {
      if (disposed) return;
      if (ready) enqueue(task);
      else pending.push(task);
    },
    async flush() {
      ready = true;
      for (const task of pending.splice(0)) enqueue(task);
      await tail;
    },
    dispose() {
      disposed = true;
      pending.length = 0;
    },
  };
}
