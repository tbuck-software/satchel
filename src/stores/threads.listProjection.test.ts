import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Thread } from '@/types/mail';

/*
 * PERF-06. The bundle list and the row list both derive from the same date
 * sections. Each used to build them itself, so one recompute of the list ran
 * the whole grouping pass twice over the same input.
 */
const sectionBuilds = vi.fn();

vi.mock('@/utils/threadList', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/utils/threadList')>();
  return {
    ...actual,
    buildThreadDateSections: (threads: Thread[]) => {
      sectionBuilds();
      return actual.buildThreadDateSections(threads);
    },
  };
});

const { useThreadsStore } = await import('./threads');

function seedThread(index: number): Thread {
  const iso = new Date(Date.now() - index * 60_000).toISOString();
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

describe('list projection', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    sectionBuilds.mockClear();
  });

  it('builds the date sections once for both projections', () => {
    const store = useThreadsStore();
    store.threads = Array.from({ length: 40 }, (_, index) => seedThread(index));

    expect(store.similarThreadGroups).toBeDefined();
    expect(store.listEntries.length).toBeGreaterThan(0);

    // Was two: one build per consumer.
    expect(sectionBuilds).toHaveBeenCalledTimes(1);
  });

  it('rebuilds them once more after the thread array changes', () => {
    const store = useThreadsStore();
    store.threads = Array.from({ length: 10 }, (_, index) => seedThread(index));
    void store.listEntries;
    void store.similarThreadGroups;
    expect(sectionBuilds).toHaveBeenCalledTimes(1);

    store.threads = Array.from({ length: 12 }, (_, index) => seedThread(index));
    void store.listEntries;
    void store.similarThreadGroups;
    expect(sectionBuilds).toHaveBeenCalledTimes(2);
  });
});
