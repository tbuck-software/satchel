/** FIFO per thread. A batch reserves all its ids without splitting its IPC call. */
export function createThreadActionQueue() {
  const tails = new Map<string, Promise<unknown>>();

  function run<T>(threadIds: readonly string[], task: () => Promise<T>): Promise<T> {
    const keys = [...new Set(threadIds)];
    const previous = [...new Set(keys.flatMap((id) => tails.has(id) ? [tails.get(id)!] : []))];
    // Start an idle lane synchronously so triage focus advances on the keypress.
    const result = previous.length > 0
      ? Promise.all(previous).then(task)
      : task();
    const settled = result.then(() => undefined, () => undefined);
    keys.forEach((id) => tails.set(id, settled));
    void settled.then(() => {
      for (const id of keys) if (tails.get(id) === settled) tails.delete(id);
    });
    return result;
  }

  return { run };
}
