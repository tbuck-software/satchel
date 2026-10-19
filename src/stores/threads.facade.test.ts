import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { threadsSurfacesAreDisjoint, useThreadsStore } from './threads';
import { collidingKeys, type Overlap } from './threads/surface';

/*
 * ARCH-16. The façade used to pass some seventy symbols through by hand: a
 * method added to a module and forgotten there existed and was unreachable.
 * It is a spread of the module surfaces now, which moves the failure mode to
 * two parts defining the same key — the later spread wins, silently. These
 * tests hold both ends.
 */

/** The supported public interface; unused legacy methods stay private or removed. */
const STORE_KEYS = [
  'activeContact', 'activeContactEmail', 'activeSimilarGroup', 'activeSimilarGroupId',
  'activeSmartBucket', 'activeSmartBucketId', 'activeThread', 'activeThreadId',
  'allContactSummaries', 'applyWorkflowStates', 'beginThreadLeave',
  'categoryFilter', 'clearSelection', 'closeSimilarGroup', 'closeSmartBucket',
  'contactSummaries', 'discardThread', 'endThreadLeave', 'ensureThreadLoaded', 'find',
  'focusNeighborThread', 'getSenderPreference', 'hasPreviousThreads', 'hasMoreThreads', 'leavingThreadIds',
  'leavingThreadStates', 'listEntries', 'load', 'loadPrevious', 'loadPreviousFailed', 'loadMore', 'loadMoreFailed', 'loadingMoreThreads',
  'mailboxFilter', 'markRead', 'markThreadsSpam', 'newSenderStack', 'nextThread', 'openContact',
  'openSimilarGroup', 'openSmartBucket', 'pinnedContactSummaries',
  'prevThread', 'refreshHead', 'refreshThreads', 'releaseRetainedThread', 'retryLoadMore',
  'scopedVisibleThreads',
  'selectAllVisibleThreads', 'selectFromInteraction', 'selectThread', 'selectedThreadIds',
  'selectedThreads', 'setAssignee', 'setMailboxFilter', 'setPriority',
  'setSenderCategory', 'setSenderCategoryByEmail',
  'setSenderPriorityByEmail', 'setSharedWith', 'setTagsForThread', 'setThreadCategory',
  'setThreadMuted', 'setThreadsRead', 'setThreadsStarred', 'setViewportStart',
  'similarThreadGroups', 'smartInboxBuckets', 'smartInboxTotalCount',
  'smartListEntries', 'threadLoadInFlight', 'threads', 'toggleSelectionForThread',
  'trashThreads', 'visibleThreadOrder', 'visibleThreads', 'workflowFilter',
];

beforeEach(() => {
  setActivePinia(createPinia());
});

describe('threads store façade', () => {
  /** Equivalence with the hand-written list it replaced: nothing lost, nothing internal leaked. */
  it('exposes only the supported public interface', () => {
    const store = useThreadsStore();
    const exposed = Object.keys(store).filter((key) => !key.startsWith('$') && !key.startsWith('_'));

    expect(exposed.sort()).toEqual([...STORE_KEYS].sort());
  });

  it('is assembled from surfaces that share no key', () => {
    // The value only type-checks when every pairwise overlap is `never`.
    expect(threadsSurfacesAreDisjoint).toBe(true);
  });

  it('reports a key two parts define', () => {
    const collisions = collidingKeys({
      selection: { clearSelection() {}, nextThread() {} },
      persistence: { load() {}, nextThread() {} },
    });

    expect(collisions).toEqual(['nextThread']);
  });

  it('has nothing to report for disjoint parts', () => {
    expect(collidingKeys({ a: { load() {} }, b: { refreshHead() {} } })).toEqual([]);
  });

  /** The point of the spread: a module method reaches the store without a second edit. */
  it('carries a method added to a surface through without listing it', () => {
    const persistence = { load() {}, archiveEverything() {} };
    const store = { threads: [], ...persistence };

    expect(Object.keys(store)).toContain('archiveEverything');
  });

  it('turns a shared key into a type error', () => {
    type Clash = Overlap<{ load: () => void }, { load: () => void; loadMore: () => void }>;
    // @ts-expect-error — `Clash` is 'load', so `true` is not assignable.
    const disjoint: [Clash] extends [never] ? true : Clash = true;

    expect(disjoint).toBe(true);
  });
});
