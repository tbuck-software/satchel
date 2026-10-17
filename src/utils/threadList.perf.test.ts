import { describe, expect, it } from 'vitest';
import type { Thread } from '@/types/mail';
import {
  buildSimilarGroups,
  buildThreadEntries,
  mergeOrdered,
  type SimilarThreadGroup,
  type ThreadListEntry,
} from '@/utils/threadList';

/*
 * PERF-06. Two costs in the same hot path: the sort comparator reparsed both
 * ISO strings on every comparison, and `buildThreadEntries` searched the whole
 * section for each bundle's lead thread.
 *
 * Both benchmarks below are interleaved A/B runs against the implementation
 * they replaced and assert the ratio. Absolute milliseconds are not a usable
 * gate here: the same measurement varies by 2-4x with machine load.
 */

function seedThread(index: number, senderIndex: number, priority: 0 | 1 | 2 = 0): Thread {
  const iso = new Date(Date.UTC(2026, 0, 1, 0, 0, 0) - index * 60_000).toISOString();
  return {
    id: `th-${index}`,
    accountId: 'acc-work',
    subject: `Betreff ${index}`,
    participants: [{ name: `Person ${senderIndex}`, email: `person${senderIndex}@example.com` }],
    snippet: 'Vorschau',
    lastMessageAt: iso,
    lastMessageAtMs: Date.parse(iso),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority,
    isStarred: false,
    hasInvite: false,
    mailboxNames: ['INBOX'],
    tags: [],
  };
}

function median(samples: number[]): number {
  const sorted = [...samples].sort((left, right) => left - right);
  return sorted[Math.floor(sorted.length / 2)] ?? 0;
}

/** The comparator before PERF-06: two `new Date` allocations per comparison. */
function compareByParsedDate(a: Thread, b: Thread): number {
  if (b.priority !== a.priority) return b.priority - a.priority;
  return new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime();
}

function compareByTimestamp(a: Thread, b: Thread): number {
  if (b.priority !== a.priority) return b.priority - a.priority;
  return b.lastMessageAtMs - a.lastMessageAtMs;
}

/** The lead assignment before PERF-06: a scan of the section per bundle. */
function buildThreadEntriesByScan(
  inputThreads: Thread[],
  similarGroups: SimilarThreadGroup[],
): ThreadListEntry[] {
  const groupByLeadThreadId = new Map<string, SimilarThreadGroup>();
  const groupedThreadIds = new Set<string>();
  similarGroups.forEach((group) => {
    const threadIds = new Set(group.threads.map((thread) => thread.id));
    const leadThread = inputThreads.find((thread) => threadIds.has(thread.id));
    if (!leadThread) return;
    groupByLeadThreadId.set(leadThread.id, group);
    group.threads.forEach((thread) => groupedThreadIds.add(thread.id));
  });

  return inputThreads.flatMap<ThreadListEntry>((thread): ThreadListEntry[] => {
    const leadGroup = groupByLeadThreadId.get(thread.id);
    if (leadGroup) return [{ kind: 'group', id: leadGroup.id, group: leadGroup }];
    if (groupedThreadIds.has(thread.id)) return [];
    return [{ kind: 'thread', id: `thread-${thread.id}`, thread }];
  });
}

describe('list sort comparator', () => {
  const source = Array.from({ length: 3000 }, (_, index) =>
    seedThread(index, index % 400, index % 97 === 0 ? 2 : 0),
  );

  it('keeps the previous order', () => {
    const withTimestamps = [...source].sort(compareByTimestamp).map((thread) => thread.id);
    const withParsedDates = [...source].sort(compareByParsedDate).map((thread) => thread.id);
    expect(withTimestamps).toEqual(withParsedDates);
  });

  it('sorts 3000 threads faster than the date-parsing comparator', () => {
    const shuffled = source.map((_, index) => source[(index * 7919) % source.length]!);
    const fast: number[] = [];
    const slow: number[] = [];
    for (let run = 0; run < 51; run += 1) {
      const copyA = [...shuffled];
      const copyB = [...shuffled];

      const start = performance.now();
      copyA.sort(compareByTimestamp);
      const midpoint = performance.now();
      copyB.sort(compareByParsedDate);
      const end = performance.now();

      if (run < 20) continue; // warmup
      fast.push(midpoint - start);
      slow.push(end - midpoint);
    }

    console.log(
      `sort(3000) timestamp median: ${median(fast).toFixed(2)} ms, `
      + `date-parsing reference: ${median(slow).toFixed(2)} ms`,
    );
    expect(median(fast) / median(slow)).toBeLessThan(0.8);
  }, 120_000);
});

describe('bundle lead assignment', () => {
  // n = 2000 rows in 200 matching subject bundles of 10.
  const section = Array.from({ length: 2000 }, (_, index) => ({
    ...seedThread(index, Math.floor(index / 10)),
    subject: `Build #${index} finished`,
  }));
  const groups = buildSimilarGroups(section);

  it('produces exactly the entries the scanning implementation produced', () => {
    expect(groups).toHaveLength(200);
    const fromMap = buildThreadEntries(section, groups).map((entry) => entry.id);
    const fromScan = buildThreadEntriesByScan(section, groups).map((entry) => entry.id);
    expect(fromMap).toEqual(fromScan);
  });

  it('beats the scanning implementation at n = 2000, g = 200', () => {
    const fast: number[] = [];
    const slow: number[] = [];
    for (let run = 0; run < 51; run += 1) {
      const start = performance.now();
      buildThreadEntries(section, groups);
      const midpoint = performance.now();
      buildThreadEntriesByScan(section, groups);
      const end = performance.now();
      if (run < 20) continue; // warmup
      fast.push(midpoint - start);
      slow.push(end - midpoint);
    }

    console.log(
      `buildThreadEntries(n=2000, g=200) median: ${median(fast).toFixed(3)} ms, `
      + `scanning reference: ${median(slow).toFixed(3)} ms`,
    );
    expect(median(fast) / median(slow)).toBeLessThan(0.8);
  }, 120_000);
});

describe('head refresh merge', () => {
  /*
   * PERF-07. The refresh sorted everything loaded on every sync event, though
   * only the fresh page had moved. Comparator calls are the honest metric:
   * n log n before, at most n + p now.
   */
  const retained = Array.from({ length: 2000 }, (_, index) => seedThread(index + 40, index % 400));
  const fresh = Array.from({ length: 40 }, (_, index) => seedThread(index, index % 40));

  it('produces the same order as sorting the concatenation', () => {
    const merged = mergeOrdered(fresh, retained, compareByTimestamp).map((thread) => thread.id);
    const sorted = [...fresh, ...retained].sort(compareByTimestamp).map((thread) => thread.id);
    expect(merged).toEqual(sorted);
  });

  it('compares fewer pairs than a full re-sort', () => {
    let mergeCalls = 0;
    let sortCalls = 0;
    mergeOrdered(fresh, retained, (a, b) => {
      mergeCalls += 1;
      return compareByTimestamp(a, b);
    });
    [...fresh, ...retained].sort((a, b) => {
      sortCalls += 1;
      return compareByTimestamp(a, b);
    });

    console.log(`refreshHead merge comparisons: ${mergeCalls}, full re-sort: ${sortCalls}`);
    expect(mergeCalls).toBeLessThanOrEqual(fresh.length + retained.length);
    expect(mergeCalls * 4).toBeLessThan(sortCalls);
  });
});
