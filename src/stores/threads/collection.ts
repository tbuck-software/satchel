import { shallowRef, type Ref } from 'vue';
import type { Thread } from '@/types/mail';
import type { WorkflowState } from '@/types/workflow';

export interface WorkflowRowMutation {
  threadId: string;
  state: WorkflowState;
  snoozeUntil?: string | null;
  /** Left untouched when omitted; a rollback restores the value it had. */
  unreadCount?: number;
}

/** `loadMore` is additive, so a long session grew the array — and every
 *  projection over it — without bound. */
export const MAX_LOADED_THREADS = 2000;

/** Three screens of context kept in front of the viewport, so a trim never
 *  removes what the user is about to scroll back to. */
const WINDOW_KEEP_AHEAD_ROWS = 120;

/** What the threads store exposes of the Collection. */
export type ThreadCollectionSurface = Pick<
  ThreadCollection,
  'threads' | 'find' | 'setViewportStart' | 'setWorkflowState' | 'applyWorkflowStates' | 'markRead'
>;

export interface ThreadCollection {
  /** The store-facing subset; the rest is for Persistence and tests. */
  surface: ThreadCollectionSurface;
  threads: Ref<Thread[]>;
  find(threadId: string): Thread | undefined;
  /** Row the virtualizer currently starts at; the trim measures against it. */
  setViewportStart(threadId: string): void;
  enforceWindow(pinnedThreadIds: readonly string[], direction?: 'forward' | 'backward'): Thread | undefined;
  /** Rebuild count of the id index. Observability for the O(1) lookup guard. */
  indexBuilds(): number;
  replaceAll(nextThreads: Thread[]): void;
  appendUnique(nextThreads: Thread[]): void;
  patchRows(patches: { id: string; values: Partial<Thread> }[]): void;
  replaceRow(threadId: string, nextThread: Thread): boolean;
  remove(threadId: string): Thread | undefined;
  setWorkflowState(threadId: string, nextState: WorkflowState, snoozeUntil?: string | null): void;
  applyWorkflowStates(mutations: WorkflowRowMutation[]): void;
  markRead(threadId: string, isRead: boolean): void;
  addLocalOutgoingThread(input: {
    threadId?: string;
    accountId: string;
    subject: string;
    recipients: string[];
    snippet: string;
  }): string;
}

/** Thread rows contain only scalars, arrays and participant records. */
function sameValue(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (Array.isArray(left) && Array.isArray(right)) {
    return left.length === right.length && left.every((value, i) => sameValue(value, right[i]));
  }
  if (!left || !right || typeof left !== 'object' || typeof right !== 'object') return false;
  const a = left as Record<string, unknown>;
  const b = right as Record<string, unknown>;
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  return [...keys].every(key => sameValue(a[key], b[key]));
}

function sameRow(left: Thread, right: Thread): boolean {
  return sameValue(left, right);
}

/**
 * Own the source thread projection and its synchronous row mutations.
 *
 * Consumers that publish a page or merge a refresh must replace the outer
 * array. This is observable by the virtualized list, whose callers do not
 * deep-watch their item arrays.
 */
export function createThreadCollection(
  initialThreads: Thread[] = [],
): ThreadCollection {
  const threads = shallowRef<Thread[]>(initialThreads);

  // Keyed on array identity rather than a dirty flag, so a caller assigning
  // `store.threads` straight through the ref cannot leave the index stale.
  const positions = new Map<string, number>();
  let indexedArray: Thread[] | null = null;
  let indexBuildCount = 0;

  function index(): Map<string, number> {
    if (indexedArray === threads.value) return positions;
    positions.clear();
    threads.value.forEach((thread, position) => positions.set(thread.id, position));
    indexedArray = threads.value;
    indexBuildCount += 1;
    return positions;
  }

  function indexBuilds(): number {
    return indexBuildCount;
  }

  function find(threadId: string): Thread | undefined {
    const position = index().get(threadId);
    return position === undefined ? undefined : threads.value[position];
  }

  function replaceAll(nextThreads: Thread[]) {
    const previous = threads.value;
    const known = index();
    const reused = nextThreads.map((thread) => {
      const position = known.get(thread.id);
      const prior = position === undefined ? undefined : previous[position];
      return prior && sameRow(prior, thread) ? prior : thread;
    });
    if (reused.length === previous.length && reused.every((row, i) => row === previous[i])) return;
    threads.value = reused;
  }

  let viewportStartId = '';
  function setViewportStart(threadId: string) {
    viewportStartId = threadId;
  }

  /**
   * Trim the front of the array back to `MAX_LOADED_THREADS`.
   *
   * Only rows far behind the viewport go; the active thread and the current
   * selection are kept in their original order and do not count as dropped,
   * so the cut walks further to still shed `overflow` rows.
   */
  function enforceWindow(pinnedThreadIds: readonly string[], direction: 'forward' | 'backward' = 'forward'): Thread | undefined {
    const overflow = threads.value.length - MAX_LOADED_THREADS;
    if (overflow <= 0) return;
    // Without a known viewport a trim would be a guess about what is on screen.
    const viewportIndex = viewportStartId ? index().get(viewportStartId) : undefined;
    if (viewportIndex === undefined) return;
    if (direction === 'backward') {
      const pinned = new Set(pinnedThreadIds);
      let dropped = 0;
      let cut = threads.value.length;
      const kept: Thread[] = [];
      while (cut > viewportIndex + WINDOW_KEEP_AHEAD_ROWS && dropped < overflow) {
        const thread = threads.value[--cut]!;
        if (pinned.has(thread.id)) kept.unshift(thread);
        else dropped += 1;
      }
      if (!dropped) return;
      const boundary = threads.value[cut - 1];
      replaceAll([...threads.value.slice(0, cut), ...kept]);
      return boundary;
    }
    const limit = viewportIndex - WINDOW_KEEP_AHEAD_ROWS;
    if (limit <= 0) return;

    const pinned = new Set(pinnedThreadIds);
    const kept: Thread[] = [];
    let dropped = 0;
    let cut = 0;
    while (cut < limit && dropped < overflow) {
      const thread = threads.value[cut]!;
      if (pinned.has(thread.id)) kept.push(thread);
      else dropped += 1;
      cut += 1;
    }
    if (dropped === 0) return;
    const boundary = threads.value[cut];
    replaceAll([...kept, ...threads.value.slice(cut)]);
    return boundary;
  }

  function appendUnique(nextThreads: Thread[]) {
    const known = index();
    const appended = nextThreads.filter((thread) => !known.has(thread.id));
    replaceAll([...threads.value, ...appended]);
  }

  function replaceRow(threadId: string, nextThread: Thread): boolean {
    const position = index().get(threadId);
    if (position === undefined) return false;
    const nextThreads = [...threads.value];
    nextThreads[position] = nextThread;
    replaceAll(nextThreads);
    return true;
  }

  function remove(threadId: string): Thread | undefined {
    const removed = find(threadId);
    replaceAll(threads.value.filter((thread) => thread.id !== threadId));
    return removed;
  }

  function setWorkflowState(threadId: string, nextState: WorkflowState, snoozeUntil?: string | null) {
    const thread = find(threadId);
    if (!thread) return;

    applyWorkflowStates([{ threadId, state: nextState, snoozeUntil }]);
  }

  /**
   * Apply a whole batch as one publication.
   *
   * Writing the rows one by one made the list and contact projections rerun
   * per thread; a bulk transition over a date section published hundreds of
   * times for a single user action.
   */
  function applyWorkflowStates(mutations: WorkflowRowMutation[]) {
    if (mutations.length === 0) return;
    const byId = new Map(mutations.map((mutation) => [mutation.threadId, mutation]));
    patchRows([...byId.values()].map((mutation) => ({
      id: mutation.threadId,
      values: {
        workflowState: mutation.state,
        snoozeUntil: mutation.state === 'snoozed'
          ? mutation.snoozeUntil || new Date(Date.now() + 2 * 60 * 60_000).toISOString()
          : undefined,
        ...(mutation.unreadCount === undefined ? {} : { unreadCount: mutation.unreadCount }),
      },
    })));
  }

  function patchRows(patches: { id: string; values: Partial<Thread> }[]) {
    const byId = new Map(patches.map(patch => [patch.id, patch.values]));
    replaceAll(threads.value.map(thread => {
      const values = byId.get(thread.id);
      return values ? { ...thread, ...values } : thread;
    }));
  }

  function markRead(threadId: string, isRead: boolean) {
    const thread = find(threadId);
    if (!thread) return;
    patchRows([{ id: threadId, values: { unreadCount: isRead ? 0 : Math.max(1, thread.unreadCount) } }]);
  }

  function addLocalOutgoingThread(input: {
    threadId?: string;
    accountId: string;
    subject: string;
    recipients: string[];
    snippet: string;
  }): string {
    const threadId = input.threadId || `th-${Math.random().toString(36).slice(2, 8)}`;
    const now = new Date().toISOString();
    const participants = input.recipients.map((email) => ({
      name: email,
      email,
    }));

    replaceAll([
      {
        id: threadId,
        accountId: input.accountId,
        subject: input.subject,
        participants: participants.length > 0 ? participants : [{ name: 'Recipient', email: 'recipient@example.com' }],
        sentParticipants: participants,
        snippet: input.snippet || input.subject,
        lastMessageAt: now,
        lastMessageAtMs: Date.parse(now),
        messageCount: 1,
        unreadCount: 0,
        hasAttachments: false,
        workflowState: 'waiting',
        priority: 1,
        mailboxNames: ['Sent'],
        tags: [],
      },
      ...threads.value.filter((thread) => thread.id !== threadId),
    ]);
    return threadId;
  }

  return {
    surface: { threads, find, setViewportStart, setWorkflowState, applyWorkflowStates, markRead },
    threads,
    find,
    indexBuilds,
    setViewportStart,
    enforceWindow,
    replaceAll,
    appendUnique,
    patchRows,
    replaceRow,
    remove,
    setWorkflowState,
    applyWorkflowStates,
    markRead,
    addLocalOutgoingThread,
  };
}
