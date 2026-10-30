import { ref } from 'vue';

/** Lists wait for writes, then retry if a write overlaps their snapshot. Only
 * the newest request owns publication, loading and errors. Kept per store/scope. */
export function orderedLoad(scope: () => string = () => '') {
  const loading = ref(false);
  const loadFailed = ref(false);
  let request = 0;
  let revision = 0;
  const writes = new Set<Promise<unknown>>();

  async function mutate<T>(write: () => Promise<T>): Promise<T> {
    revision++;
    const pending = write();
    writes.add(pending);
    try {
      return await pending;
    } finally {
      revision++;
      writes.delete(pending);
    }
  }

  async function load<T>(read: () => Promise<T>, publish: (rows: T) => void,
    failed?: (error: unknown) => void, succeeded?: () => void) {
    const id = ++request;
    const startedScope = scope();
    const current = () => id === request && startedScope === scope();
    loading.value = true;
    loadFailed.value = false;
    try {
      while (current()) {
        if (writes.size > 0) {
          await Promise.allSettled(writes);
          if (!current()) return;
          if (writes.size > 0) continue;
        }
        const version = revision;
        try {
          const rows = await read();
          if (!current()) return;
          if (version !== revision || writes.size > 0) continue;
          publish(rows);
          succeeded?.();
          return;
        } catch (error) {
          if (!current()) return;
          if (version !== revision || writes.size > 0) continue;
          loadFailed.value = true;
          failed?.(error);
          return;
        }
      }
    } finally {
      if (id === request) loading.value = false;
    }
  }

  return { loading, loadFailed, load, mutate };
}
