import { watch } from 'vue';
import { defineStore } from 'pinia';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useAccountsStore } from '@/stores/accounts';
import { useSettingsStore } from '@/stores/settings';
import { recordFrontendDiagnostic } from '@/utils/diagnostics';
import { useConnectionStore } from '@/stores/connection';
import {
  createThreadCollection,
  type ThreadCollectionSurface,
} from '@/stores/threads/collection';
import {
  createThreadFilterBuckets,
  type ThreadFilterBucketsSurface,
} from '@/stores/threads/filter-buckets';
import { createThreadLoadState } from '@/stores/threads/load-state';
import {
  createThreadPersistence,
  type ThreadPersistenceSurface,
} from '@/stores/threads/persistence';
import {
  createThreadSelection,
  createVisibleThreadProjection,
  type ThreadSelectionSurface,
} from '@/stores/threads/selection';
import type { Overlap } from '@/stores/threads/surface';

/*
 * Construction order is the dependency order, and each step gets the finished
 * objects before it: Collection → Selection → Filter+Buckets → Persistence.
 * The two edges that point backwards are not arguments. Selection reads the
 * visible order through a projection Filter+Buckets fills in, and a scope
 * change reaches Persistence through a listener subscribed once it exists.
 */
export const useThreadsStore = defineStore('threads', () => {
  const accountsStore = useAccountsStore();
  const mailboxesStore = useMailboxesStore();
  const settingsStore = useSettingsStore();
  const connectionStore = useConnectionStore();

  const collection = createThreadCollection();
  const loadState = createThreadLoadState();
  const projection = createVisibleThreadProjection();

  const selection = createThreadSelection({
    threads: collection.threads,
    find: collection.find,
    adopt: collection.appendUnique,
    projection,
  });
  const selectionBridge = {
    getActiveThreadId: () => selection.surface.activeThreadId.value,
    setActiveThreadId: (threadId: string) => {
      selection.surface.activeThreadId.value = threadId;
    },
    getSelectedThreadIds: () => selection.surface.selectedThreadIds.value,
    clearSelection: () => selection.surface.clearSelection(),
  };

  const filterBuckets = createThreadFilterBuckets({
    threads: collection.threads,
    accounts: () => accountsStore.accounts,
    activeAccountId: () => accountsStore.activeAccountId,
    isAccountVisible: (id) => accountsStore.isVisible(id),
    inboxSenderBuckets: () => settingsStore.inboxSenderBuckets,
    acceptedSenders: () => settingsStore.acceptedSenders,
    acceptedDomains: () => settingsStore.acceptedDomains,
    senderReviewStartedAt: () => settingsStore.senderReviewStartedAt,
    dismissedNewSenders: () => settingsStore.dismissedNewSenders,
    pinnedContacts: () => settingsStore.pinnedContacts,
    inboxGroupingMode: () => settingsStore.inboxGroupingMode,
    loadState,
    selection: selectionBridge,
    projection,
    recordDiagnostic: recordFrontendDiagnostic,
  });

  /** Drop a projection row that the backend confirms no longer exists. */
  function discardThread(threadId: string) {
    collection.remove(threadId);
    selection.reconcileRemovedThread(threadId);
  }

  const persistence = createThreadPersistence({
    collection,
    accounts: () => accountsStore.accounts,
    activeAccountId: () => accountsStore.activeAccountId,
    filter: filterBuckets.filter,
    loadState,
    selection: selectionBridge,
    settings: {
      dismissedNewSenders: () => settingsStore.dismissedNewSenders,
      setDismissedNewSender: settingsStore.setDismissedNewSender,
      setSenderInboxBucket: settingsStore.setSenderInboxBucket,
    },
    connection: {
      clearBackendError: connectionStore.clearBackendError,
      reportBackendError: connectionStore.reportBackendError,
    },
    reloadMailboxes: () => mailboxesStore.load(),
    adjustMailboxUnread: (changes) => mailboxesStore.adjustUnreadThreads(changes),
    discardThread,
  });
  filterBuckets.onScopeChange(() => void persistence.surface.load());

  watch(() => accountsStore.visibilityKey, () => {
    loadState.smartInboxSummary.value = null;
    selection.surface.clearSelection();
    if (!accountsStore.activeAccountId) selection.surface.activeThreadId.value = '';
    void persistence.surface.load();
  });
  filterBuckets.startWatching();
  selection.startWatching();

  return {
    ...collection.surface,
    ...selection.surface,
    ...filterBuckets.surface,
    ...persistence.surface,
    discardThread,
  };
});

// A later spread silently wins over an earlier one. Any key two parts share
// turns this into a type error naming that key.
type SurfaceOverlap =
  | Overlap<ThreadCollectionSurface, ThreadSelectionSurface>
  | Overlap<ThreadCollectionSurface, ThreadFilterBucketsSurface>
  | Overlap<ThreadCollectionSurface, ThreadPersistenceSurface>
  | Overlap<ThreadSelectionSurface, ThreadFilterBucketsSurface>
  | Overlap<ThreadSelectionSurface, ThreadPersistenceSurface>
  | Overlap<ThreadFilterBucketsSurface, ThreadPersistenceSurface>
  | Overlap<
    ThreadCollectionSurface & ThreadSelectionSurface & ThreadFilterBucketsSurface & ThreadPersistenceSurface,
    { discardThread: unknown }
  >;
export const threadsSurfacesAreDisjoint: [SurfaceOverlap] extends [never] ? true : SurfaceOverlap =
  true;

export type {
  ContactThreadSummary,
  MailboxFilter,
  NewSenderDomainSuggestion,
  NewSenderGroup,
  NewSenderStack,
  SimilarThreadGroup,
  SmartInboxBucketGroup,
  SmartInboxBucketKey,
  SmartInboxBucketPreviewItem,
  SmartInboxBucketSummary,
  ThreadListEntry,
  SelectionModifiers,
} from '@/utils/threadList';
