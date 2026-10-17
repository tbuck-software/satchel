import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import type { Thread } from '@/types/mail';
import { createThreadCollection } from '@/stores/threads/collection';
import { useThreadsStore } from './threads';

/*
 * PERF-04. `setSelection` used to call a linear `find` per id, so `Cmd+A` was
 * quadratic in the loaded thread count.
 *
 * Wall time on this machine swings by 2-4x under load, so the benchmark below
 * interleaves the indexed lookup with the linear reference it replaced and
 * asserts the ratio between the two, never an absolute duration.
 */

// 1500, not the 3000 of the issue: the linear reference is quadratic, and at
// 3000 a single reference run costs seconds. The claim is the ratio, not N.
const N = 1500;

function seedThread(index: number): Thread {
  const iso = new Date(Date.UTC(2026, 0, 1, 0, 0, index % 60)).toISOString();
  return {
    id: `th-${index}`,
    accountId: 'acc-work',
    subject: `Betreff ${index}`,
    participants: [{ name: `Person ${index}`, email: `person${index}@example.com` }],
    snippet: 'Vorschau',
    lastMessageAt: iso,
    lastMessageAtMs: Date.parse(iso),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    isStarred: false,
    hasInvite: false,
    mailboxNames: ['INBOX'],
    tags: [],
  };
}

function seedThreads(count: number): Thread[] {
  return Array.from({ length: count }, (_, index) => seedThread(index));
}

/** The pre-PERF-04 lookup: a full scan of the array per id. */
function linearSelection(source: readonly Thread[], threadIds: readonly string[]): string[] {
  const unique = Array.from(new Set(threadIds));
  return unique.filter((id) => Boolean(source.find((thread) => thread.id === id)));
}

function median(samples: number[]): number {
  const sorted = [...samples].sort((left, right) => left - right);
  return sorted[Math.floor(sorted.length / 2)] ?? 0;
}

describe('thread collection index', () => {
  it('rebuilds the id index once per publication, not once per lookup', () => {
    const collection = createThreadCollection(seedThreads(N));

    const before = collection.indexBuilds();
    for (let index = 0; index < N; index += 1) {
      expect(collection.find(`th-${index}`)?.id).toBe(`th-${index}`);
    }

    expect(collection.indexBuilds() - before).toBe(1);

    collection.replaceAll(seedThreads(10));
    expect(collection.find('th-5')?.id).toBe('th-5');
    expect(collection.indexBuilds() - before).toBe(2);
  });

  it('keeps lookups correct after a row is replaced or removed', () => {
    const collection = createThreadCollection(seedThreads(5));

    expect(collection.replaceRow('th-2', { ...seedThread(2), subject: 'Neu' })).toBe(true);
    expect(collection.find('th-2')?.subject).toBe('Neu');

    collection.remove('th-2');
    expect(collection.find('th-2')).toBeUndefined();
    expect(collection.find('th-3')?.id).toBe('th-3');

    collection.appendUnique([seedThread(3), seedThread(99)]);
    expect(collection.threads.value).toHaveLength(5);
    expect(collection.find('th-99')?.id).toBe('th-99');
  });
});

describe('select-all over a large loaded window', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it('selects every visible thread in order', () => {
    const store = useThreadsStore();
    store.threads = seedThreads(50);

    store.selectAllVisibleThreads();

    expect(store.selectedThreadIds).toEqual(store.visibleThreadOrder);
    expect(store.selectedThreadIds).toHaveLength(50);
  });

  it('drops ids that are not loaded', () => {
    const store = useThreadsStore();
    store.threads = seedThreads(5);

    store.setSelection(['th-1', 'th-nope', 'th-3', 'th-1']);

    expect(store.selectedThreadIds).toEqual(['th-1', 'th-3']);
  });

  it('beats the linear reference lookup', () => {
    const store = useThreadsStore();
    store.threads = seedThreads(N);
    const order = store.visibleThreadOrder;
    expect(order).toHaveLength(N);

    const indexed: number[] = [];
    const linear: number[] = [];
    for (let run = 0; run < 7; run += 1) {
      const startIndexed = performance.now();
      store.setSelection(order);
      const midpoint = performance.now();
      linearSelection(store.threads, order);
      const end = performance.now();
      if (run < 2) continue; // warmup
      indexed.push(midpoint - startIndexed);
      linear.push(end - midpoint);
    }

    expect(store.selectedThreadIds).toHaveLength(N);
    console.log(
      `setSelection(${N}) indexed median: ${median(indexed).toFixed(2)} ms, `
      + `linear reference: ${median(linear).toFixed(2)} ms`,
    );
    // Ratio, not milliseconds: the same run is 2-4x slower on a loaded machine.
    expect(median(indexed) / median(linear)).toBeLessThan(0.8);
  }, 120_000);
});
