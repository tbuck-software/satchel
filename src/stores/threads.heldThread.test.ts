import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { beforeEach, describe, expect, it } from 'vitest';
import type { Thread } from '@/types/mail';
import type { WorkflowState } from '@/types/workflow';
import { useThreadsStore } from './threads';

function thread(index: number, workflowState: WorkflowState = 'inbox'): Thread {
  const iso = new Date(Date.UTC(2026, 10, 23, 10, 60 - index)).toISOString();
  return {
    id: `th-${index}`,
    accountId: 'acc-work',
    subject: `Subject ${index}`,
    participants: [{ name: `Person ${index}`, email: `person${index}@example.com` }],
    snippet: 'Preview',
    lastMessageAt: iso,
    lastMessageAtMs: Date.parse(iso),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState,
    priority: 0,
    isStarred: false,
    hasInvite: false,
    mailboxNames: ['INBOX'],
    tags: [],
  };
}

/** A change the reader did not make: a send filing the thread, another device, a sync. */
function changeBehindTheReader(store: ReturnType<typeof useThreadsStore>, id: string, state: WorkflowState) {
  store.threads = store.threads.map((row) => (row.id === id ? { ...row, workflowState: state } : row));
}

/** What a click does: select the row, and the reader shows it. */
function open(store: ReturnType<typeof useThreadsStore>, id: string) {
  store.selectThread(id);
  store.showInReader(id);
}

const visibleIds = (store: ReturnType<typeof useThreadsStore>) => store.visibleThreads.map((row) => row.id);

describe('the open thread under a change made elsewhere', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it('stays open and listed when it leaves the Inbox behind the reader', async () => {
    const store = useThreadsStore();
    store.threads = [thread(0), thread(1), thread(2)];
    open(store, 'th-1');
    await nextTick();
    expect(store.activeThreadLeftList).toBe(false);

    changeBehindTheReader(store, 'th-1', 'done');
    await nextTick();

    // It used to jump to the top of the list while the reply was on screen.
    expect(store.activeThreadId).toBe('th-1');
    expect(visibleIds(store)).toEqual(['th-0', 'th-1', 'th-2']);
    expect(store.activeThreadLeftList).toBe(true);
  });

  it('lets the row go once the reader moves on', async () => {
    const store = useThreadsStore();
    store.threads = [thread(0), thread(1), thread(2)];
    open(store, 'th-1');
    await nextTick();
    changeBehindTheReader(store, 'th-1', 'done');
    await nextTick();

    store.nextThread();
    store.showInReader(store.activeThreadId);
    await nextTick();

    expect(store.activeThreadId).toBe('th-2');
    expect(visibleIds(store)).toEqual(['th-0', 'th-2']);
    expect(store.activeThreadLeftList).toBe(false);
  });

  it('lets the row go when the list changes scope', async () => {
    const store = useThreadsStore();
    store.threads = [thread(0), thread(1)];
    open(store, 'th-1');
    await nextTick();
    changeBehindTheReader(store, 'th-1', 'done');
    await nextTick();

    store.workflowFilter = 'later';
    await nextTick();

    expect(visibleIds(store)).not.toContain('th-1');
  });

  it('keeps holding when the thread comes back while open', async () => {
    const store = useThreadsStore();
    store.threads = [thread(0), thread(1)];
    open(store, 'th-1');
    await nextTick();
    changeBehindTheReader(store, 'th-1', 'done');
    await nextTick();

    changeBehindTheReader(store, 'th-1', 'inbox');
    await nextTick();

    expect(store.activeThreadLeftList).toBe(false);
    expect(visibleIds(store)).toEqual(['th-0', 'th-1']);
  });

  it('holds nothing the reader does not show, like the selection behind a phone list', async () => {
    const store = useThreadsStore();
    store.threads = [thread(0), thread(1)];
    store.selectThread('th-1');
    await nextTick();

    changeBehindTheReader(store, 'th-1', 'done');
    await nextTick();

    expect(visibleIds(store)).toEqual(['th-0']);
  });

  it('lets the row go when the reader closes', async () => {
    const store = useThreadsStore();
    store.threads = [thread(0), thread(1)];
    open(store, 'th-1');
    await nextTick();
    changeBehindTheReader(store, 'th-1', 'done');
    await nextTick();

    store.closeReader('th-1');
    await nextTick();

    expect(visibleIds(store)).toEqual(['th-0']);
  });

  it('never draws in a thread this list did not show', async () => {
    const store = useThreadsStore();
    store.threads = [thread(0), thread(1), thread(9, 'done')];
    await nextTick();

    open(store, 'th-9');
    await nextTick();

    expect(visibleIds(store)).toEqual(['th-0', 'th-1']);
  });
});
