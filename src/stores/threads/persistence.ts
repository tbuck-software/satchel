import { computed, ref, type Ref } from 'vue';
import {
  backendCommandsAvailable,
  backendDismissMobileNotifications,
  backendGetSenderPreference,
  backendGetThread,
  backendGetThreads,
  backendListCachedThreads,
  backendListSmartInboxSummary,
  backendListThreads,
  backendMarkThreadsSpam,
  backendSetSenderPreference,
  backendSetThreadCategory,
  backendSetThreadMuted,
  backendSetThreadPriority,
  backendSetThreadsStarred,
  backendSetThreadTags,
  backendSetThreadsRead,
  backendTrashThreads,
  backendUpdateThreadOrganization,
  type BackendListThreadsInput,
  type BackendSenderPreference,
  type BackendThreadCursor,
  type BackendThreadPage,
  type BackendTrashReceipt,
} from '@/services/backend';
import type { Account, Thread } from '@/types/mail';
import type { InboxSenderBucket } from '@/stores/settings';
import type { ThreadCollection } from '@/stores/threads/collection';
import type { ThreadFilterBuckets } from '@/stores/threads/filter-buckets';
import {
  smartInboxSummaryScopeKey,
  threadListInput,
  type ThreadListScope,
  type ThreadLoadState,
} from '@/stores/threads/load-state';
import {
  mergeOrdered,
  primarySenderEmail,
  toThreadModel,
} from '@/utils/threadList';

/** The filter state and projections Persistence reads. Built before it. */
type PersistenceFilter = Pick<
  ThreadFilterBuckets,
  | 'activeContactEmail'
  | 'workflowFilter'
  | 'mailboxFilter'
  | 'activeSmartBucketId'
  | 'visibleThreads'
  | 'newSenderStack'
  | 'compareThreads'
>;

interface PersistenceSelectionBridge {
  getActiveThreadId(): string;
  setActiveThreadId(threadId: string): void;
  getSelectedThreadIds(): readonly string[];
}

interface PersistenceSettingsBridge {
  dismissedNewSenders(): Record<string, string>;
  setDismissedNewSender(email: string, throughDate: string | null): void;
  setSenderInboxBucket(email: string, bucket: InboxSenderBucket): void;
}

interface PersistenceConnectionBridge {
  clearBackendError(source: string): void;
  reportBackendError(source: string, error: unknown): void;
}

interface ThreadPersistenceDependencies {
  collection: ThreadCollection;
  accounts: () => readonly Account[];
  activeAccountId: () => string;
  filter: PersistenceFilter;
  loadState: ThreadLoadState;
  selection: PersistenceSelectionBridge;
  settings: PersistenceSettingsBridge;
  connection: PersistenceConnectionBridge;
  reloadMailboxes(): void | Promise<void>;
  /** Optimistic sidebar badges; `delta` is per thread that flipped. */
  adjustMailboxUnread(
    changes: Array<{ accountId: string; mailboxNames: string[]; workflowState: string; delta: number }>,
  ): void;
  discardThread(threadId: string): void;
}

interface EnsureThreadLoadedOptions {
  /** Keep a directly opened detail row when an Inbox load replaces its page. */
  retainAcrossListLoads?: boolean;
}

/** What the threads store exposes of Persistence. */
export interface ThreadPersistenceSurface {
  threadLoadInFlight: Ref<boolean>;
  hasMoreThreads: Ref<boolean>;
  loadingMoreThreads: Ref<boolean>;
  /** The next page failed; the list is not complete, and says so. */
  loadMoreFailed: Ref<boolean>;
  loadPreviousFailed: Ref<boolean>;
  hasPreviousThreads: Ref<boolean>;
  ensureThreadLoaded(threadId: string, options?: EnsureThreadLoadedOptions): Promise<void>;
  releaseRetainedThread(threadId: string): void;
  setPriority(threadId: string, priority: Thread['priority']): Promise<void>;
  setThreadMuted(threadId: string, muted: boolean): Promise<void>;
  setThreadsStarred(threadIds: string[], starred: boolean): Promise<void>;
  setThreadCategory(threadId: string, category: Thread['category']): Promise<void>;
  setSenderCategoryByEmail(
    accountId: string,
    senderEmail: string,
    category: NonNullable<Thread['category']>,
    bucket: InboxSenderBucket,
  ): Promise<void>;
  getSenderPreference(senderEmail: string): Promise<BackendSenderPreference>;
  setSenderPriorityByEmail(
    accountId: string,
    senderEmail: string,
    isPriority: boolean,
  ): Promise<void>;
  setSenderCategory(
    threadId: string,
    category: NonNullable<Thread['category']>,
    bucket: InboxSenderBucket,
  ): Promise<void>;
  markThreadsSpam(threadIds: string[]): Promise<BackendTrashReceipt>;
  setThreadsRead(threadIds: string[], isRead: boolean): Promise<void>;
  trashThreads(threadIds: string[]): Promise<BackendTrashReceipt>;
  setTagsForThread(threadId: string, tags: string[]): Promise<void>;
  setAssignee(threadId: string, assignee?: string): Promise<void>;
  setSharedWith(threadId: string, sharedWith: string[]): Promise<void>;
  load(): Promise<void>;
  /** Merge the head without moving the tail cursor. During load, schedule one catch-up and return. */
  refreshHead(options?: { revalidateCachedRows?: boolean }): Promise<void>;
  refreshThreads(threadIds: string[]): Promise<void>;
  loadMore(): Promise<void>;
  loadPrevious(): Promise<void>;
  retryLoadMore(): Promise<void>;
}

export interface ThreadPersistence {
  surface: ThreadPersistenceSurface;
}

// Enough for several phone viewports. Remaining rows use the existing keyset
// pagination and arrive as the user approaches the end of the loaded list.
const THREAD_PAGE_SIZE = 40;

/** One pass over the array instead of one per outcome. */
function partitionThreads(
  source: readonly Thread[],
  targetIds: ReadonlySet<string>,
): { removed: Thread[]; retained: Thread[] } {
  const removed: Thread[] = [];
  const retained: Thread[] = [];
  for (const thread of source) {
    if (targetIds.has(thread.id)) removed.push(thread);
    else retained.push(thread);
  }
  return { removed, retained };
}

/**
 * Own backend-backed thread state, pagination, cache publication, and remote
 * mutations. The factory receives narrow bridges instead of Pinia stores so
 * the persistence seam remains usable behind the threads façade.
 */
export function createThreadPersistence({
  collection,
  accounts,
  activeAccountId,
  filter,
  loadState,
  selection,
  settings,
  connection,
  reloadMailboxes,
  adjustMailboxUnread,
  discardThread,
}: ThreadPersistenceDependencies): ThreadPersistence {
  const threads = collection.threads;
  const { threadLoadInFlight, smartInboxSummary, smartInboxSummaryInputKey } = loadState;
  const threadPageCursor = ref<BackendThreadCursor | null>(null);
  const hasMoreThreads = ref(false);
  const loadingMoreThreads = ref(false);
  const loadMoreFailed = ref(false);
  let windowStartCursor: BackendThreadCursor | null = null;
  const hasPreviousThreads = ref(false);
  const failedPageDirection = ref<'forward' | 'backward'>('forward');
  const loadPreviousFailed = computed(() => loadMoreFailed.value && failedPageDirection.value === 'backward');
  let loadRequestId = 0;
  let refreshRequestId = 0;
  let pendingHeadRefresh = false;
  let pendingRowRevalidation = false;
  let cursorInputKey = '';
  let activeThreadLoadCount = 0;
  let idlePrefetchHandle: number | null = null;
  let idlePrefetchUsesBrowserIdleCallback = false;
  let retainedDetailThreadId = '';

  function currentScope(): ThreadListScope {
    return {
      accountId: activeAccountId(),
      activeContactEmail: filter.activeContactEmail.value,
      workflowFilter: filter.workflowFilter.value,
      mailboxFilter: filter.mailboxFilter.value,
      activeSmartBucketId: filter.activeSmartBucketId.value,
    };
  }

  /** The exact backend scope used by list, cache, and keyset requests. */
  function buildBackendThreadListInput(): BackendListThreadsInput {
    return threadListInput(currentScope(), THREAD_PAGE_SIZE);
  }

  function threadListInputKey(input: BackendListThreadsInput): string {
    return JSON.stringify({
      account_id: input.account_id ?? '',
      contact_email: input.contact_email ?? '',
      workflow_state: input.workflow_state ?? '',
      mailbox_kind: input.mailbox_kind ?? '',
      mailbox_value: input.mailbox_value ?? '',
      smart_bucket: input.smart_bucket ?? '',
    });
  }

  /**
   * Make sure a thread exists in the store — search hits often point at
   * conversations outside the loaded page; without the row the detail pane
   * has nothing to render.
   */
  async function ensureThreadLoaded(
    threadId: string,
    options: EnsureThreadLoadedOptions = {},
  ) {
    if (options.retainAcrossListLoads) {
      retainedDetailThreadId = threadId;
    }
    if (!threadId || threads.value.some((thread) => thread.id === threadId)) return;
    if (!backendCommandsAvailable()) return;
    try {
      const dto = await backendGetThread(threadId);
      if (!threads.value.some((thread) => thread.id === dto.id)) {
        collection.appendUnique([toThreadModel(dto, accounts())]);
      }
    } catch {
      // The hit may point at a thread deleted meanwhile; the pane shows its
      // own fallback in that case.
    }
  }

  function releaseRetainedThread(threadId: string) {
    if (retainedDetailThreadId === threadId) {
      retainedDetailThreadId = '';
    }
  }

  // Serialize optimistic mutations so two failed writes cannot restore each
  // other's uncommitted values. Reads and list loading remain independent.
  let mutationTail: Promise<unknown> = Promise.resolve();
  function serializeMutation<T>(run: () => Promise<T>): Promise<T> {
    const result = mutationTail.then(run);
    mutationTail = result.catch(() => {});
    return result;
  }

  /**
   * `alwaysRollBack` keeps the contract of writes that used to roll back and
   * rethrow even without a backend, instead of keeping the local echo.
   */
  async function writeOptimistically(
    patches: { id: string; values: Partial<Thread> }[],
    write: () => Promise<unknown>,
    { alwaysRollBack = false }: { alwaysRollBack?: boolean } = {},
  ) {
    return serializeMutation(async () => {
      const previous = patches.flatMap(({ id, values }) => {
        const thread = collection.find(id);
        if (!thread) return [];
        const before = Object.fromEntries(Object.keys(values).map((key) => [key, thread[key as keyof Thread]]));
        // Publish the whole optimistic batch after taking its snapshot.
        return [{ id, before }];
      });
      collection.patchRows(patches);
      try {
        await write();
      } catch (error) {
        if (!alwaysRollBack && !backendCommandsAvailable()) return;
        collection.patchRows(previous.map(({ id, before }) => ({ id, values: before })));
        throw error;
      }
    });
  }

  /**
   * Drop rows from the list before a remote move and return the rollback.
   * The rollback only reinserts into the list the rows were taken from: after
   * a scope change they belong to another projection, so the list is
   * refreshed instead.
   */
  function removeOptimistically(targets: ReadonlySet<string>): () => void {
    const scopeKey = threadListInputKey(buildBackendThreadListInput());
    const { removed, retained } = partitionThreads(threads.value, targets);
    collection.replaceAll(retained);
    return () => {
      if (scopeKey !== threadListInputKey(buildBackendThreadListInput())) {
        void refreshHead();
        return;
      }
      const present = new Set(threads.value.map((thread) => thread.id));
      collection.replaceAll([...threads.value, ...removed.filter((thread) => !present.has(thread.id))]
        .sort(filter.compareThreads));
    };
  }

  async function setPriority(threadId: string, priority: Thread['priority']) {
    if (!collection.find(threadId)) return;
    await writeOptimistically([{ id: threadId, values: { priority } }],
      () => backendSetThreadPriority(threadId, priority), { alwaysRollBack: true });
  }

  async function setThreadMuted(threadId: string, muted: boolean) {
    await writeOptimistically([{ id: threadId, values: {
      isMuted: muted, ...(muted ? { unreadCount: 0 } : {}),
    } }], () => backendSetThreadMuted(threadId, muted));
  }

  /**
   * Starring is a selection action, so the store owns the batch: one optimistic
   * write, one round trip, one rollback if the command refuses the batch.
   */
  async function setThreadsStarred(threadIds: string[], starred: boolean) {
    const normalized = Array.from(new Set(threadIds.map((id) => id.trim()).filter(Boolean)));
    if (normalized.length === 0) return;
    await writeOptimistically(
      normalized.map((id) => ({ id, values: { isStarred: starred } })),
      () => backendSetThreadsStarred(normalized, starred),
    );
  }

  async function setThreadCategory(threadId: string, category: Thread['category']) {
    await writeOptimistically([{ id: threadId, values: { category, smartBucket: undefined } }],
      () => backendSetThreadCategory(threadId, category ?? null));
  }

  /**
   * A sender preference is about the person, not the mailbox they wrote to —
   * the local echo therefore ignores `accountId`, matching the backend.
   */
  async function setSenderCategoryByEmail(
    accountId: string,
    senderEmail: string,
    category: NonNullable<Thread['category']>,
    bucket: InboxSenderBucket,
  ) {
    const normalized = senderEmail.trim().toLowerCase();
    if (!normalized) return;
    try {
      await backendSetSenderPreference({ accountId, senderEmail: normalized, category });
      settings.setSenderInboxBucket(normalized, bucket);
      if (backendCommandsAvailable()) {
        await load();
        return;
      }
    } catch (error) {
      if (backendCommandsAvailable()) throw error;
      settings.setSenderInboxBucket(normalized, bucket);
    }
    for (const candidate of threads.value) {
      if (primarySenderEmail(candidate) === normalized) {
        candidate.category = category;
        candidate.smartBucket = undefined;
      }
    }
  }

  async function getSenderPreference(senderEmail: string): Promise<BackendSenderPreference> {
    const normalized = senderEmail.trim().toLowerCase();
    return backendGetSenderPreference(normalized);
  }

  async function setSenderPriorityByEmail(
    accountId: string,
    senderEmail: string,
    isPriority: boolean,
  ) {
    const normalized = senderEmail.trim().toLowerCase();
    if (!normalized) return;
    try {
      await backendSetSenderPreference({ accountId, senderEmail: normalized, isPriority });
      if (backendCommandsAvailable()) {
        await load();
        return;
      }
    } catch (error) {
      if (backendCommandsAvailable()) throw error;
    }
    for (const candidate of threads.value) {
      if (primarySenderEmail(candidate) === normalized) {
        candidate.priority = isPriority ? 2 : 1;
      }
    }
  }

  async function setSenderCategory(
    threadId: string,
    category: NonNullable<Thread['category']>,
    bucket: InboxSenderBucket,
  ) {
    const thread = collection.find(threadId);
    const senderEmail = thread ? primarySenderEmail(thread) : '';
    if (thread && senderEmail) {
      await setSenderCategoryByEmail(thread.accountId, senderEmail, category, bucket);
    } else {
      await setThreadCategory(threadId, category);
    }
  }

  async function markThreadsSpam(threadIds: string[]): Promise<BackendTrashReceipt> {
    const targets = new Set(threadIds.map((id) => id.trim()).filter(Boolean));
    const normalized = [...targets];
    const empty: BackendTrashReceipt = { undo_token: null, undo_thread_ids: [], permanently_deleted: [] };
    if (normalized.length === 0) return empty;
    const receipt = await serializeMutation(async () => {
      const rollback = removeOptimistically(targets);
      try {
        return await backendMarkThreadsSpam(normalized);
      } catch (error) {
        if (!backendCommandsAvailable()) return empty;
        rollback();
        throw error;
      }
    });
    // A refresh failure cannot roll back a committed action or lose its Undo receipt.
    await Promise.allSettled([refreshHead(), reloadMailboxes()]);
    return receipt;
  }

  /*
   * Reading costs one write per thread. The list, its summary, the sidebar
   * badges and the notification dismissal settle once the reader pauses,
   * not after every message: reading 200 mails was about 1,000 round trips.
   */
  const READ_RECONCILE_MS = 1_500;
  let readReconcileTimer: ReturnType<typeof setTimeout> | null = null;
  const readDismissals = new Set<string>();

  function scheduleReadReconcile() {
    if (readReconcileTimer) clearTimeout(readReconcileTimer);
    readReconcileTimer = setTimeout(() => {
      readReconcileTimer = null;
      const dismissed = [...readDismissals];
      readDismissals.clear();
      if (dismissed.length > 0) void backendDismissMobileNotifications(dismissed).catch(() => {});
      void Promise.allSettled([refreshHead(), reloadMailboxes()]);
    }, READ_RECONCILE_MS);
  }

  function unreadDelta(before: number, after: number): number {
    return (after > 0 ? 1 : 0) - (before > 0 ? 1 : 0);
  }

  /** Bucket badges in the Inbox summary count unread messages, not threads. */
  function adjustSummaryUnread(changes: Array<{ thread: Thread; before: number; after: number }>) {
    const summary = smartInboxSummary.value;
    if (!summary) return;
    const byId = new Map(changes.map((change) => [change.thread.id, change]));
    smartInboxSummary.value = {
      ...summary,
      buckets: summary.buckets.map((bucket) => {
        let delta = 0;
        const previewItems = bucket.preview_items.map((item) => {
          const change = byId.get(item.id);
          return change ? { ...item, unread_count: change.after } : item;
        });
        for (const change of changes) {
          if (change.thread.smartBucket === bucket.key) delta += change.after - change.before;
        }
        return { ...bucket, unread_count: Math.max(0, bucket.unread_count + delta), preview_items: previewItems };
      }),
    };
  }

  async function setThreadsRead(threadIds: string[], isRead: boolean) {
    const normalized = Array.from(new Set(threadIds.map((id) => id.trim()).filter(Boolean)));
    if (normalized.length === 0) return;
    // Read the previous count inside the serialized mutation, after any earlier
    // mutation has either committed or rolled back.
    await serializeMutation(async () => {
      const changes = normalized.flatMap((id) => {
        const thread = collection.find(id);
        if (!thread) return [];
        const after = isRead ? 0 : Math.max(1, thread.unreadCount);
        return [{ thread, before: thread.unreadCount, after }];
      });
      const badgeChanges = (direction: 1 | -1) => changes
        .map(({ thread, before, after }) => ({
          accountId: thread.accountId,
          mailboxNames: thread.mailboxNames,
          workflowState: thread.workflowState,
          delta: direction * unreadDelta(before, after),
        }))
        .filter((change) => change.delta !== 0);
      collection.patchRows(changes.map(({ thread, after }) => ({
        id: thread.id, values: { unreadCount: after },
      })));
      adjustMailboxUnread(badgeChanges(1));
      adjustSummaryUnread(changes);
      try {
        await backendSetThreadsRead(normalized, isRead);
      } catch (error) {
        if (!backendCommandsAvailable()) return;
        collection.patchRows(changes.map(({ thread, before }) => ({ id: thread.id, values: { unreadCount: before } })));
        adjustMailboxUnread(badgeChanges(-1));
        adjustSummaryUnread(changes.map((change) => ({ ...change, before: change.after, after: change.before })));
        throw error;
      }
    });
    if (isRead) normalized.forEach((id) => readDismissals.add(id));
    scheduleReadReconcile();
  }

  async function trashThreads(threadIds: string[]) {
    const targets = new Set(threadIds.map((threadId) => threadId.trim()).filter(Boolean));
    const normalized = [...targets];
    if (normalized.length === 0) {
      return { undo_token: null, undo_thread_ids: [], permanently_deleted: [] };
    }

    const dismissed = (filter.newSenderStack.value?.senders ?? [])
      .filter((sender) => sender.threadIds.some((threadId) => targets.has(threadId)))
      .map((sender) => ({
        email: sender.senderEmail,
        previous: settings.dismissedNewSenders()[sender.senderEmail],
        through: sender.lastMessageAt,
      }));
    dismissed.forEach((sender) => settings.setDismissedNewSender(sender.email, sender.through));

    const rollback = removeOptimistically(targets);
    let receipt: BackendTrashReceipt;
    try {
      receipt = await backendTrashThreads(normalized);
    } catch (error) {
      dismissed.forEach((sender) => settings.setDismissedNewSender(sender.email, sender.previous ?? null));
      rollback();
      throw error;
    }
    // A refresh failure cannot roll back a committed action or lose its Undo receipt.
    await Promise.allSettled([backendDismissMobileNotifications(normalized), refreshHead(), reloadMailboxes()]);
    return receipt;
  }

  async function setTagsForThread(threadId: string, tags: string[]) {
    if (!collection.find(threadId)) return;
    const normalized = Array.from(
      new Set(tags.map((tag) => tag.trim().toLowerCase()).filter(Boolean)),
    );
    await writeOptimistically([{ id: threadId, values: { tags: normalized } }],
      () => backendSetThreadTags([threadId], normalized), { alwaysRollBack: true });
  }

  /*
   * The organization command writes the whole row. Serialized, each write
   * reads the row only after the previous one committed or rolled back, so a
   * refusal cannot restore a value the backend has already replaced.
   */
  async function setAssignee(threadId: string, assignee?: string) {
    if (!collection.find(threadId)) return;
    const normalized = assignee?.trim();
    await writeOptimistically([{ id: threadId, values: { assignee: normalized ? normalized : undefined } }],
      () => persistThreadOrganization(threadId), { alwaysRollBack: true });
  }

  async function setSharedWith(threadId: string, sharedWith: string[]) {
    if (!collection.find(threadId)) return;
    const normalized = Array.from(
      new Set(
        sharedWith
          .map((value) => value.trim())
          .filter((value) => value.length > 0),
      ),
    );
    await writeOptimistically([{ id: threadId, values: { sharedWith: normalized } }],
      () => persistThreadOrganization(threadId), { alwaysRollBack: true });
  }

  function persistThreadOrganization(threadId: string) {
    const thread = collection.find(threadId);
    if (!thread) return Promise.resolve();
    return backendUpdateThreadOrganization({
      threadId: thread.id,
      boardPosition: thread.boardPosition ?? null,
      assignee: thread.assignee ?? null,
      sharedWith: thread.sharedWith ?? [],
    });
  }

  function publishThreadPage(
    page: BackendThreadPage,
    input: BackendListThreadsInput,
  ) {
    const fresh = page.threads.map((row) => toThreadModel(row, accounts()));
    const retainedDetail = retainedDetailThreadId
      ? collection.find(retainedDetailThreadId)
      : undefined;
    const nextThreads = input.account_id
      ? [...fresh, ...threads.value.filter((thread) => thread.accountId !== input.account_id)]
      : fresh;
    // A notification may open an archived/done conversation while this Inbox
    // request is already in flight. Its row is intentionally absent from the
    // Inbox response, but the routed detail must remain readable. Keeping only
    // the explicitly retained detail avoids leaking stale rows into the list.
    if (retainedDetail && !nextThreads.some((thread) => thread.id === retainedDetail.id)) {
      nextThreads.push(retainedDetail);
    }
    collection.replaceAll(nextThreads);
    cursorInputKey = threadListInputKey(input);
    threadPageCursor.value = page.next_cursor;
    windowStartCursor = null;
    hasPreviousThreads.value = false;
    hasMoreThreads.value = page.has_more;
    loadMoreFailed.value = false;
    const visible = filter.visibleThreads.value;
    if (!visible.some((thread) => thread.id === selection.getActiveThreadId())) {
      selection.setActiveThreadId(visible[0]?.id ?? '');
    }
  }

  function cancelIdlePrefetch() {
    if (idlePrefetchHandle === null || typeof window === 'undefined') return;
    const idleWindow = window as unknown as {
      cancelIdleCallback?: (handle: number) => void;
    };
    if (idlePrefetchUsesBrowserIdleCallback && idleWindow.cancelIdleCallback) {
      idleWindow.cancelIdleCallback(idlePrefetchHandle);
    } else {
      window.clearTimeout(idlePrefetchHandle);
    }
    idlePrefetchHandle = null;
  }

  function scheduleIdlePrefetch(requestId: number) {
    cancelIdlePrefetch();
    if (
      typeof window === 'undefined'
      || !backendCommandsAvailable()
      || !hasMoreThreads.value
      || !threadPageCursor.value
    ) {
      return;
    }

    const inputKey = threadListInputKey(buildBackendThreadListInput());
    const run = () => {
      idlePrefetchHandle = null;
      if (
        requestId !== loadRequestId
        || inputKey !== threadListInputKey(buildBackendThreadListInput())
      ) {
        return;
      }
      void loadMore();
    };

    const idleWindow = window as unknown as {
      requestIdleCallback?: (callback: () => void, options: { timeout: number }) => number;
    };
    if (idleWindow.requestIdleCallback) {
      idlePrefetchUsesBrowserIdleCallback = true;
      idlePrefetchHandle = idleWindow.requestIdleCallback(run, { timeout: 1_500 });
    } else {
      idlePrefetchUsesBrowserIdleCallback = false;
      idlePrefetchHandle = globalThis.setTimeout(run, 750) as unknown as number;
    }
  }

  /**
   * The bucket counts in the smart folder row come from the server summary.
   * Every refresh that changes thread state has to renew it, or the counts
   * freeze on the pre-action numbers.
   */
  async function refreshSmartInboxSummary(isCurrent: () => boolean) {
    const inputKey = smartInboxSummaryScopeKey(currentScope());
    const activeContactEmail = filter.activeContactEmail.value;
    const mailboxFilter = filter.mailboxFilter.value;
    if (
      !activeContactEmail
      && mailboxFilter.kind === 'section'
      && mailboxFilter.value === 'inbox'
    ) {
      try {
        const summary = await backendListSmartInboxSummary(activeAccountId() || null);
        if (!isCurrent()) return;
        smartInboxSummary.value = summary;
        smartInboxSummaryInputKey.value = inputKey;
      } catch {
        // Keep the last summary for this scope on a transient failure.
      }
    } else {
      smartInboxSummary.value = null;
      smartInboxSummaryInputKey.value = '';
    }
  }

  async function load() {
    const requestId = ++loadRequestId;
    loadingMoreThreads.value = false;
    ++refreshRequestId;
    const input = buildBackendThreadListInput();
    cancelIdlePrefetch();
    activeThreadLoadCount += 1;
    threadLoadInFlight.value = true;
    try {
      // Revalidation begins immediately. Wrapping its rejection keeps it
      // handled while the tiny cached projection is read in parallel.
      const freshPage = backendListThreads(input).then(
        (page) => ({ page, error: null }),
        (error: unknown) => ({ page: null, error }),
      );
      if (backendCommandsAvailable()) {
        try {
          const cachedPage = await backendListCachedThreads(input);
          if (requestId !== loadRequestId) return;
          if (cachedPage) {
            publishThreadPage(cachedPage, input);
          }
        } catch (error) {
          console.warn('Failed to load cached threads', error);
        }
      }

      const freshResult = await freshPage;
      if (!freshResult.page) throw freshResult.error;
      const page = freshResult.page;
      // Any newer load supersedes this one, even with identical filters:
      // archiving a selection fires one reload per thread, and an early
      // response landing last would put the just-archived threads back on
      // screen until the next refresh.
      if (requestId !== loadRequestId) {
        return;
      }
      // Backend reachable → its result is the truth, including "no threads".
      // Keeping the mock seed on an empty result would show fake mail in a
      // freshly set-up (or fully archived) real account.
      // Threads of OTHER accounts stay cached: switching accounts then shows
      // the last known list instantly (visibleThreads filters client-side)
      // while this load refreshes the switched-to account in the background.
      publishThreadPage(page, input);
      connection.clearBackendError('threads');
      // Bucket counts are secondary metadata. Publish the actual mail rows
      // before running their three aggregate queries so first paint does not
      // wait behind Smart Inbox bookkeeping.
      await refreshSmartInboxSummary(() => requestId === loadRequestId);
      if (requestId === loadRequestId) {
        scheduleIdlePrefetch(requestId);
      }
    } catch (err) {
      if (requestId !== loadRequestId) {
        return;
      }
      console.error('Failed to load threads', err);
      if (backendCommandsAvailable()) {
        connection.reportBackendError('threads', err);
      }
    } finally {
      activeThreadLoadCount = Math.max(0, activeThreadLoadCount - 1);
      threadLoadInFlight.value = activeThreadLoadCount > 0;
      if (!threadLoadInFlight.value && pendingHeadRefresh) {
        pendingHeadRefresh = false;
        const revalidateCachedRows = pendingRowRevalidation;
        pendingRowRevalidation = false;
        void refreshHead({ revalidateCachedRows });
      }
    }
  }

  /**
   * Re-read the newest page and merge it into what is already loaded.
   *
   * This is the refresh for *incoming* mail, as opposed to `load()`, which is
   * the refresh for a changed filter. `load()` replaces the array with page one
   * and resets the cursor — so firing it on every `mail:incoming` threw anyone
   * who had scrolled past the first page back to the newest 200 threads and
   * dropped `hasMoreThreads`, undoing exactly what `loadMore()` is careful to
   * preserve. Merging keeps the loaded tail, the scroll position and the
   * selection intact.
   *
   * Sync, resume and lag recovery can revalidate cached rows outside the head.
   * Ordinary incoming-mail refreshes rely on the thread update feed for them.
   *
   * Falls back to `load()` when nothing is loaded yet, so the first refresh
   * after a failed initial load still establishes a cursor.
   */
  async function refreshHead(options: { revalidateCachedRows?: boolean } = {}) {
    // A load owns page one and its cursor; catch up once that load settles.
    if (threadLoadInFlight.value) {
      pendingHeadRefresh = true;
      pendingRowRevalidation ||= options.revalidateCachedRows === true;
      return;
    }
    if (!backendCommandsAvailable()) {
      return load();
    }
    if (threads.value.length === 0) {
      return load();
    }

    const requestId = ++refreshRequestId;
    const loadGeneration = loadRequestId;
    const isCurrent = () => requestId === refreshRequestId && loadGeneration === loadRequestId;
    const input = buildBackendThreadListInput();
    const inputKey = threadListInputKey(input);
    try {
      const snapshot = new Map(threads.value.map(thread => [thread.id, thread]));
      const page = await backendListThreads(input);
      const headIds = new Set(page.threads.map(thread => thread.id));
      const tailIds = options.revalidateCachedRows
        ? [...snapshot.values()]
          .filter(thread => (!input.account_id || thread.accountId === input.account_id) && !headIds.has(thread.id))
          .map(thread => thread.id)
        : [];
      const tailRows = tailIds.length ? await backendGetThreads(tailIds) : [];
      // A filter change (or a full load) started while we were fetching — that
      // one owns the array now.
      if (!isCurrent()
        || inputKey !== threadListInputKey(buildBackendThreadListInput())) {
        return;
      }

      const compare = filter.compareThreads;
      const fresh = page.threads.flatMap(row => {
        const current = collection.find(row.id);
        if (snapshot.has(row.id) && snapshot.get(row.id) !== current) return current ? [current] : [];
        return [toThreadModel(row, accounts())];
      }).sort(compare);
      const refreshed = new Set(fresh.map((thread) => thread.id));
      // Revalidate the tail too: a resumed mobile webview can miss moves and deletions.
      const revalidated = new Map(tailRows.map(row => [row.id, toThreadModel(row, accounts())]));
      const queried = new Set(tailIds);
      const retained = threads.value.flatMap(thread => {
        if (refreshed.has(thread.id)) return [];
        if (!queried.has(thread.id) || snapshot.get(thread.id) !== thread) return [thread];
        const updated = revalidated.get(thread.id);
        return updated ? [updated] : [];
      }).sort(compare);
      // Merge the refreshed head with the revalidated tail without resetting its cursor.
      collection.replaceAll(mergeOrdered(fresh, retained, compare));
      connection.clearBackendError('threads');
      await refreshSmartInboxSummary(isCurrent);

      // Deliberately NOT touching threadPageCursor/hasMoreThreads: they describe
      // how far the tail has been paged, which this refresh does not change.
      const visible = filter.visibleThreads.value;
      if (!visible.some((thread) => thread.id === selection.getActiveThreadId())) {
        selection.setActiveThreadId(visible[0]?.id ?? '');
      }
    } catch (err) {
      if (!isCurrent()) {
        return;
      }
      // Transient failures keep the last successful projection visible.
      console.error('Failed to refresh threads', err);
      connection.reportBackendError('threads', err);
    }
  }

  /** Revalidate a burst of cached rows with one IPC and one publication. */
  async function refreshThreads(threadIds: string[]) {
    const snapshot = new Map([...new Set(threadIds)].flatMap(id => {
      const thread = collection.find(id);
      return thread ? [[id, thread] as const] : [];
    }));
    if (!snapshot.size) return;
    const generation = loadRequestId;
    try {
      const rows = await backendGetThreads([...snapshot.keys()]);
      if (generation !== loadRequestId) return;
      const updated = new Map(rows.map(row => [row.id, toThreadModel(row, accounts())]));
      const removed: string[] = [];
      const next = threads.value.flatMap(thread => {
        if (snapshot.get(thread.id) !== thread) return [thread];
        const row = updated.get(thread.id);
        if (!row) removed.push(thread.id);
        return row ? [row] : [];
      });
      collection.replaceAll(next.sort(filter.compareThreads));
      removed.forEach(id => discardThread(id));
      connection.clearBackendError('threads');
    } catch (error) {
      connection.reportBackendError('threads', error);
    }
  }

  /**
   * Fetch the next keyset page and append it.
   *
   * Deliberately additive: replacing the array would drop the user's scroll
   * position and any selection made in earlier pages. Additive but capped —
   * `enforceWindow` trims the far front once the array passes
   * `MAX_LOADED_THREADS`.
   */
  async function loadMore() {
    // A cursor only exists once a real backend answered, so no extra
    // native-mode guard is needed here.
    // After a failure only the explicit retry pages again; scrolling would
    // otherwise hammer the same failing cursor.
    if (
      threadLoadInFlight.value
      || cursorInputKey !== threadListInputKey(buildBackendThreadListInput())
      || !hasMoreThreads.value
      || loadingMoreThreads.value
      || loadMoreFailed.value
      || !threadPageCursor.value
    ) {
      return;
    }

    const requestId = loadRequestId;
    const input = { ...buildBackendThreadListInput(), cursor: threadPageCursor.value };
    const inputKey = threadListInputKey(input);
    loadingMoreThreads.value = true;
    try {
      const page = await backendListThreads(input);
      // A filter change or a full reload happened while we were fetching.
      if (requestId !== loadRequestId
        || inputKey !== threadListInputKey(buildBackendThreadListInput())) {
        return;
      }

      const known = new Set(threads.value.map((thread) => thread.id));
      const appended = page.threads
        .map((row) => toThreadModel(row, accounts()))
        .filter((thread) => !known.has(thread.id));

      collection.appendUnique(appended);
      // Trimming happens after the append, so the cursor keeps describing how
      // far the tail has been paged — the trim only touches the front.
      const boundary = collection.enforceWindow([
        selection.getActiveThreadId(),
        ...selection.getSelectedThreadIds(),
      ]);
      if (boundary) {
        windowStartCursor = { id: boundary.id, last_message_date: boundary.sortDate ?? boundary.lastMessageAt };
        hasPreviousThreads.value = true;
      }
      threadPageCursor.value = page.next_cursor;
      hasMoreThreads.value = page.has_more;
    } catch (err) {
      if (requestId !== loadRequestId || inputKey !== threadListInputKey(buildBackendThreadListInput())) return;
      console.error('Failed to load more threads', err);
      // The cursor and hasMoreThreads stay: the list is incomplete, not done.
      failedPageDirection.value = 'forward';
      loadMoreFailed.value = true;
    } finally {
      if (requestId === loadRequestId) loadingMoreThreads.value = false;
    }
  }

  async function loadPrevious() {
    if (!hasPreviousThreads.value || !windowStartCursor || loadingMoreThreads.value || loadMoreFailed.value || threadLoadInFlight.value
      || cursorInputKey !== threadListInputKey(buildBackendThreadListInput())) return;
    const requestId = loadRequestId;
    const input = { ...buildBackendThreadListInput(), before_cursor: windowStartCursor };
    const inputKey = threadListInputKey(input);
    loadingMoreThreads.value = true;
    try {
      const page = await backendListThreads(input);
      if (requestId !== loadRequestId || inputKey !== threadListInputKey(buildBackendThreadListInput())) return;
      const fresh = page.threads.map(row => toThreadModel(row, accounts()));
      const ids = new Set(fresh.map(row => row.id));
      collection.replaceAll([...fresh, ...threads.value.filter(row => !ids.has(row.id))]
        .sort((a, b) => Date.parse(b.sortDate ?? b.lastMessageAt) - Date.parse(a.sortDate ?? a.lastMessageAt)
          || b.id.localeCompare(a.id)));
      const boundary = collection.enforceWindow([
        selection.getActiveThreadId(), ...selection.getSelectedThreadIds(),
      ], 'backward');
      if (boundary) {
        threadPageCursor.value = { id: boundary.id, last_message_date: boundary.sortDate ?? boundary.lastMessageAt };
        hasMoreThreads.value = true;
      }
      if (fresh[0]) windowStartCursor = { id: fresh[0].id, last_message_date: fresh[0].sortDate ?? fresh[0].lastMessageAt };
      hasPreviousThreads.value = page.has_more;
    } catch (err) {
      if (requestId !== loadRequestId || inputKey !== threadListInputKey(buildBackendThreadListInput())) return;
      console.error('Failed to load previous threads', err);
      failedPageDirection.value = 'backward';
      loadMoreFailed.value = true;
    } finally {
      if (requestId === loadRequestId) loadingMoreThreads.value = false;
    }
  }

  async function retryLoadMore() {
    const direction = failedPageDirection.value;
    loadMoreFailed.value = false;
    if (direction === 'backward') await loadPrevious();
    else await loadMore();
  }

  return {
    surface: {
      threadLoadInFlight,
      hasMoreThreads,
      loadingMoreThreads,
      loadMoreFailed,
      loadPreviousFailed,
      hasPreviousThreads,
      ensureThreadLoaded,
      releaseRetainedThread,
      setPriority,
      setThreadMuted,
      setThreadsStarred,
      setThreadCategory,
      setSenderCategoryByEmail,
      getSenderPreference,
      setSenderPriorityByEmail,
      setSenderCategory,
      markThreadsSpam,
      setThreadsRead,
      trashThreads,
      setTagsForThread,
      setAssignee,
      setSharedWith,
      load,
      refreshHead,
      refreshThreads,
      loadMore,
      loadPrevious,
      retryLoadMore,
    },
  };
}
