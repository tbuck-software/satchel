import { computed, ref, watch, type ComputedRef, type Ref } from 'vue';
import { backendCommandsAvailable } from '@/services/backend';
import type { InboxGroupingMode, InboxSenderBucket } from '@/stores/settings';
import type { Account, Thread } from '@/types/mail';
import { smartInboxSummaryScopeKey, type ThreadLoadState } from '@/stores/threads/load-state';
import type { VisibleThreadProjection } from '@/stores/threads/selection';
import { matchesWorkflowFilter, type WorkflowFilter } from '@/types/workflow';
import {
  buildContactSummaries,
  buildLaterSections,
  compareLaterThreads,
  buildTopContactSummaries,
  buildDateGroupedEntries,
  buildDateScopedSimilarGroups,
  buildThreadDateSections,
  sectionEntries,
  sectionSimilarGroups,
  buildNewSenderStack,
  buildPinnedContactSummaries,
  buildPriorityCardEntry,
  buildSmartDateGroupedEntries,
  buildSmartInboxBuckets,
  cleanThreadSnippet,
  collectNewSenderThreadIds,
  isVisibleInCurrentScope,
  matchesMailboxFilter,
  normalizedParticipantEmail,
  normalizedSubjectWhitespace,
  threadHasParticipant,
  type ContactThreadSummary,
  type MailboxFilter,
  type NewSenderStack,
  type SimilarThreadGroup,
  type SmartInboxBucketGroup,
  type ThreadListEntry,
} from '@/utils/threadList';

/** Rows the sidebar's contact section fills beyond the pinned ones. */
export const FREQUENT_CONTACT_LIMIT = 5;

interface ThreadSelectionBridge {
  getActiveThreadId(): string;
  setActiveThreadId(threadId: string): void;
  clearSelection(): void;
}

interface ThreadFilterDependencies {
  threads: Ref<Thread[]>;
  accounts: () => readonly Account[];
  activeAccountId: () => string;
  isAccountVisible?: (accountId: string) => boolean;
  inboxSenderBuckets: () => Record<string, InboxSenderBucket>;
  acceptedSenders: () => Record<string, true>;
  acceptedDomains: () => Record<string, true>;
  senderReviewStartedAt: () => string | null;
  dismissedNewSenders: () => Record<string, string>;
  pinnedContacts: () => readonly string[];
  inboxGroupingMode: () => InboxGroupingMode;
  loadState: ThreadLoadState;
  selection: ThreadSelectionBridge;
  /** Filled here so Selection, built earlier, can read the visible order. */
  projection: VisibleThreadProjection;
  recordDiagnostic?: (event: string, detail: string) => void;
}

/** What the threads store exposes of Filter+Buckets. */
export interface ThreadFilterBucketsSurface {
  activeSimilarGroupId: Ref<string>;
  activeSmartBucketId: Ref<string>;
  workflowFilter: Ref<WorkflowFilter>;
  mailboxFilter: Ref<MailboxFilter>;
  categoryFilter: Ref<'all' | 'personal' | 'updates' | 'promotions' | 'social'>;
  activeContactEmail: Ref<string>;
  activeSimilarGroup: ComputedRef<SimilarThreadGroup | undefined>;
  activeSmartBucket: ComputedRef<SmartInboxBucketGroup | undefined>;
  activeContact: ComputedRef<ContactThreadSummary | null>;
  visibleThreads: ComputedRef<Thread[]>;
  contactSummaries: ComputedRef<ContactThreadSummary[]>;
  frequentContactSummaries: ComputedRef<ContactThreadSummary[]>;
  allContactSummaries: ComputedRef<ContactThreadSummary[]>;
  pinnedContactSummaries: ComputedRef<Map<string, ContactThreadSummary>>;
  similarThreadGroups: ComputedRef<SimilarThreadGroup[]>;
  newSenderStack: ComputedRef<NewSenderStack | null>;
  smartInboxBuckets: ComputedRef<SmartInboxBucketGroup[]>;
  smartInboxTotalCount: ComputedRef<number>;
  smartListEntries: ComputedRef<ThreadListEntry[]>;
  scopedVisibleThreads: ComputedRef<Thread[]>;
  listEntries: ComputedRef<ThreadListEntry[]>;
  visibleThreadOrder: ComputedRef<string[]>;
  openSimilarGroup(groupId: string): void;
  closeSimilarGroup(): void;
  openSmartBucket(bucketId: string): void;
  closeSmartBucket(): void;
  openContact(email: string): void;
  setMailboxFilter(next: MailboxFilter): void;
}

export interface ThreadFilterBuckets extends ThreadFilterBucketsSurface {
  compareThreads(a: Thread, b: Thread): number;
}

export interface ThreadFilterBucketsModule {
  surface: ThreadFilterBucketsSurface;
  /** Readable by Persistence, which is built after this module. */
  filter: ThreadFilterBuckets;
  /**
   * Called whenever the scope changes in a way the loaded page no longer
   * answers. Persistence subscribes once it exists; this module does not know
   * it.
   */
  onScopeChange(listener: () => void): void;
  startWatching(): void;
}

/**
 * Own thread scope state and the projections derived from it.
 *
 * This is deliberately a plain factory rather than a Pinia store. The
 * façade supplies source data and narrow callbacks for selection, reload, and
 * backend capability checks, so this module has no store or backend seam of
 * its own.
 */
export function createThreadFilterBuckets({
  threads,
  accounts,
  activeAccountId,
  isAccountVisible = () => true,
  inboxSenderBuckets,
  acceptedSenders,
  acceptedDomains,
  senderReviewStartedAt,
  dismissedNewSenders,
  pinnedContacts,
  inboxGroupingMode,
  loadState,
  selection,
  projection,
  recordDiagnostic,
}: ThreadFilterDependencies): ThreadFilterBucketsModule {
  const { smartInboxSummary, smartInboxSummaryInputKey, threadLoadInFlight } = loadState;
  const scopeListeners: Array<() => void> = [];
  function reload() {
    for (const listener of scopeListeners) listener();
  }
  const activeSimilarGroupId = ref('');
  const openedSimilarGroup = ref<SimilarThreadGroup | null>(null);
  const activeSmartBucketId = ref('');
  const workflowFilter = ref<WorkflowFilter>('all');
  const mailboxFilter = ref<MailboxFilter>({ kind: 'section', value: 'inbox' });
  const categoryFilter = ref<'all' | 'personal' | 'updates' | 'promotions' | 'social'>('all');
  const activeContactEmail = ref('');

  const ownAccountEmails = computed(
    () => new Set(accounts().map((account) => normalizedParticipantEmail(account.email)).filter(Boolean)),
  );
  const accountScopedThreads = computed(() =>
    activeAccountId()
      ? threads.value.filter((thread) => thread.accountId === activeAccountId())
      : threads.value.filter(thread => isAccountVisible(thread.accountId)),
  );
  const contactSummaries = computed(() =>
    buildContactSummaries(accountScopedThreads.value, ownAccountEmails.value),
  );
  /**
   * The head of `contactSummaries` without pinned addresses, for the
   * always-mounted sidebar. It needs five rows; the full projection sorted
   * every known address for them on each thread mutation.
   */
  const frequentContactSummaries = computed(() =>
    buildTopContactSummaries(
      accountScopedThreads.value,
      ownAccountEmails.value,
      FREQUENT_CONTACT_LIMIT,
      new Set(pinnedContacts()),
    ),
  );
  /**
   * Unscoped variant, for the contacts module only.
   *
   * Anything that is mounted on every route must use a narrower projection:
   * this one walks every loaded thread a second time.
   */
  const attentionThreads = computed(() => threads.value.filter(thread => isAccountVisible(thread.accountId) || thread.accountId === activeAccountId()));
  const allContactSummaries = computed(() =>
    buildContactSummaries(attentionThreads.value, ownAccountEmails.value),
  );
  /** Pins are cross-account, so their names resolve over every account. */
  const pinnedContactSummaries = computed(() =>
    buildPinnedContactSummaries(attentionThreads.value, ownAccountEmails.value, pinnedContacts()),
  );
  // Read on every Inbox render; without the guard it kept the full contact
  // projection evaluated when no contact filter was set.
  const activeContact = computed(() =>
    activeContactEmail.value
      ? contactSummaries.value.find((contact) => contact.email === activeContactEmail.value) ?? null
      : null,
  );

  const visibleThreads = computed(() => {
    const accountId = activeAccountId();
    const contactEmail = normalizedParticipantEmail(activeContactEmail.value);
    const visible: Thread[] = [];

    // This projection reruns for every mailbox mutation. Keep it to one pass:
    // chained filters allocated and traversed up to six intermediate arrays
    // before the virtual list could render the same result.
    for (const thread of threads.value) {
      if (!accountId && !isAccountVisible(thread.accountId)) continue;
      if (accountId && thread.accountId !== accountId) continue;

      if (contactEmail) {
        if (threadHasParticipant(thread, contactEmail)) visible.push(thread);
        continue;
      }

      if (!matchesWorkflowFilter(thread.workflowState, workflowFilter.value)) continue;
      if (!matchesMailboxFilter(thread, mailboxFilter.value)) continue;
      if (!isVisibleInCurrentScope(thread, workflowFilter.value, mailboxFilter.value)) continue;
      // Treat threads without a category as 'personal' so the personal tab feels right out of the box.
      if (categoryFilter.value !== 'all' && (thread.category ?? 'personal') !== categoryFilter.value) continue;
      // Muted threads stay accessible via folder views but are hidden from the
      // default Inbox to keep noise down.
      if (
        thread.isMuted
        && mailboxFilter.value.kind === 'section'
        && mailboxFilter.value.value === 'inbox'
      ) continue;

      visible.push(thread);
    }

    if (contactEmail) return visible.sort((a, b) => b.lastMessageAtMs - a.lastMessageAtMs);
    return visible.sort(workflowFilter.value === 'later' ? compareLaterThreads : compareThreads);
  });

  /**
   * The one fixed order: newest first, with priority mails lifted. Combined
   * with the date sections this bumps important mails to the top of their
   * section while the list as a whole stays chronological.
   */
  function compareThreads(a: Thread, b: Thread): number {
    if (b.priority !== a.priority) return b.priority - a.priority;
    return b.lastMessageAtMs - a.lastMessageAtMs;
  }

  /*
   * One projection feeds both the bundle list and the row list. Building the
   * date sections per consumer ran the whole grouping pass twice per recompute.
   */
  const visibleDateSections = computed(() =>
    workflowFilter.value === 'later' && !activeContactEmail.value
      ? buildLaterSections(visibleThreads.value)
      : buildThreadDateSections(visibleThreads.value),
  );
  const similarThreadGroups = computed<SimilarThreadGroup[]>(() =>
    sectionSimilarGroups(visibleDateSections.value),
  );
  /*
   * The review stack and the smart buckets split the visible threads along
   * the same line. Each used to run the sender test over every thread, so a
   * single-row patch paid for it twice.
   */
  const newSenderThreadIds = computed(() =>
    collectNewSenderThreadIds(visibleThreads.value, {
      senderBuckets: inboxSenderBuckets(),
      acceptedSenders: acceptedSenders(),
      acceptedDomains: acceptedDomains(),
      senderReviewStartedAt: senderReviewStartedAt(),
      dismissedNewSenders: dismissedNewSenders(),
      ownEmails: ownAccountEmails.value,
    }),
  );
  const newSenderStack = computed<NewSenderStack | null>(() =>
    buildNewSenderStack(visibleThreads.value, newSenderThreadIds.value),
  );
  // Summary rows are scoped to account + mailbox. Ignore an earlier scope as
  // soon as navigation changes instead of briefly labelling the new Inbox
  // with counts and previews from the previous one.
  const currentSmartInboxSummary = computed(() =>
    smartInboxSummaryInputKey.value === smartInboxSummaryScopeKey({
      accountId: activeAccountId(),
      activeContactEmail: activeContactEmail.value,
      workflowFilter: workflowFilter.value,
      mailboxFilter: mailboxFilter.value,
      activeSmartBucketId: activeSmartBucketId.value,
    })
      ? smartInboxSummary.value
      : null,
  );
  const smartInboxTotalCount = computed(
    () => currentSmartInboxSummary.value?.total_count ?? visibleThreads.value.length,
  );
  const smartInboxBuckets = computed<SmartInboxBucketGroup[]>(() =>
    buildSmartInboxBuckets(
      visibleThreads.value,
      newSenderThreadIds.value,
      currentSmartInboxSummary.value?.buckets,
    ),
  );
  const smartInboxRemainderThreads = computed(() => {
    const reservedThreadIds = new Set(
      smartInboxBuckets.value.flatMap((bucket) => bucket.threadIds),
    );
    newSenderStack.value?.threadIds.forEach((threadId) => reservedThreadIds.add(threadId));
    return visibleThreads.value.filter((thread) => !reservedThreadIds.has(thread.id));
  });

  const activeSmartBucket = computed(() => {
    if (!activeSmartBucketId.value) {
      return undefined;
    }
    return smartInboxBuckets.value.find(
      (entry) => entry.id === activeSmartBucketId.value,
    );
  });
  const similarGroupScopeThreads = computed(() => {
    if (activeSmartBucket.value) {
      return activeSmartBucket.value.threads;
    }
    if (
      mailboxFilter.value.kind !== 'folder'
      && !activeContactEmail.value
      && inboxGroupingMode() === 'smart'
    ) {
      return smartInboxRemainderThreads.value;
    }
    return visibleThreads.value;
  });
  const availableSimilarGroups = computed(() =>
    similarGroupScopeThreads.value === visibleThreads.value
      ? similarThreadGroups.value
      : buildDateScopedSimilarGroups(similarGroupScopeThreads.value),
  );
  const activeSimilarGroup = computed(() => {
    if (!activeSimilarGroupId.value) {
      return undefined;
    }

    const opened = openedSimilarGroup.value;
    if (!opened || opened.id !== activeSimilarGroupId.value) {
      return availableSimilarGroups.value.find((group) => group.id === activeSimilarGroupId.value);
    }

    // Keep an opened bundle anchored to the members the user saw. It remains
    // open for its last member, and newly synced mail cannot join a bulk action
    // the user started before that mail arrived.
    const memberIds = new Set(opened.threads.map((thread) => thread.id));
    const remaining = similarGroupScopeThreads.value.filter((thread) => memberIds.has(thread.id));
    if (remaining.length === 0) {
      return undefined;
    }

    const latest = remaining[0];
    if (!latest) return undefined;
    const sender = latest.participants[0];
    return {
      ...opened,
      senderName: sender?.name || sender?.email || opened.senderName,
      senderEmail: sender?.email || opened.senderEmail,
      title: normalizedSubjectWhitespace(latest.normalizedSubject ?? latest.subject) || latest.subject,
      previewText: cleanThreadSnippet(latest.snippet, latest.subject),
      count: remaining.length,
      unreadCount: remaining.reduce((sum, thread) => sum + thread.unreadCount, 0),
      lastMessageAt: latest.lastMessageAt,
      threads: remaining,
    };
  });

  const scopedVisibleThreads = computed(() =>
    activeSimilarGroup.value?.threads ?? activeSmartBucket.value?.threads ?? visibleThreads.value,
  );

  const listEntries = computed<ThreadListEntry[]>(() => {
    if (activeSimilarGroup.value) {
      return activeSimilarGroup.value.threads.map((thread) => ({
        kind: 'thread',
        id: `thread-${thread.id}`,
        thread,
      }));
    }

    return sectionEntries(visibleDateSections.value);
  });

  const smartListEntries = computed<ThreadListEntry[]>(() => {
    if (activeSimilarGroup.value) {
      return listEntries.value;
    }

    if (activeSmartBucket.value) {
      return buildDateGroupedEntries(activeSmartBucket.value.threads);
    }

    const entries: ThreadListEntry[] = [];
    if (newSenderStack.value) {
      entries.push({
        kind: 'new-sender-stack',
        id: newSenderStack.value.id,
        stack: newSenderStack.value,
      });
    }

    // Priority leaves the folder section and leads the list as full rows;
    // the remaining folders group under "Heute" between today's plain mails
    // and the older date sections.
    //
    // The card only earns that place when it has rows to lead with. Its count
    // comes from the server summary and its rows from the loaded window, and
    // the two disagree whenever the SQL classifier sees a thread the local one
    // does not. Without the guard the card renders a header, a rule and
    // "+9 more" with nothing in between — a fold over an empty card. A bucket
    // without rows belongs on the folder row, which is honest about being a
    // link, and opening it asks the server for the threads directly.
    const priorityBucket = smartInboxBuckets.value.find((bucket) => bucket.key === 'priority');
    const priorityLeadsWithRows = (priorityBucket?.threads.length ?? 0) > 0;
    if (priorityBucket && priorityLeadsWithRows) {
      entries.push(buildPriorityCardEntry(priorityBucket));
    }

    const folderBuckets = smartInboxBuckets.value.filter(
      (bucket) => bucket.key !== 'priority' || !priorityLeadsWithRows,
    );
    const folderRow = folderBuckets.length > 0
      ? { kind: 'folder-row' as const, id: 'smart-folders', folders: folderBuckets }
      : null;

    entries.push(...buildSmartDateGroupedEntries(smartInboxRemainderThreads.value, folderRow));

    return entries;
  });

  const visibleThreadOrder = computed(() => scopedVisibleThreads.value.map((thread) => thread.id));

  function openSimilarGroup(groupId: string) {
    const group = availableSimilarGroups.value.find((entry) => entry.id === groupId);
    if (!group) {
      return;
    }
    openedSimilarGroup.value = group;
    activeSimilarGroupId.value = groupId;
    selection.clearSelection();

    if (!group.threads.some((thread) => thread.id === selection.getActiveThreadId())) {
      selection.setActiveThreadId(group.threads[0]?.id ?? '');
    }
  }

  function closeSimilarGroup() {
    activeSimilarGroupId.value = '';
    openedSimilarGroup.value = null;
    selection.clearSelection();
  }

  function openSmartBucket(bucketId: string) {
    activeSmartBucketId.value = bucketId;
    activeSimilarGroupId.value = '';
    openedSimilarGroup.value = null;
    selection.clearSelection();
    const bucket = smartInboxBuckets.value.find((entry) => entry.id === bucketId);
    if (!bucket) return;

    if (!bucket.threads.some((thread) => thread.id === selection.getActiveThreadId())) {
      selection.setActiveThreadId(bucket.threads[0]?.id ?? '');
    }
    if (backendCommandsAvailable()) reload();
  }

  function closeSmartBucket() {
    activeSmartBucketId.value = '';
    activeSimilarGroupId.value = '';
    openedSimilarGroup.value = null;
    selection.clearSelection();
    if (backendCommandsAvailable()) reload();
  }

  function openContact(email: string) {
    const normalized = normalizedParticipantEmail(email);
    if (!normalized) {
      return;
    }

    recordDiagnostic?.('contact.open', `email=${normalized}`);
    activeContactEmail.value = normalized;
    workflowFilter.value = 'all';
    mailboxFilter.value = { kind: 'all', value: null };
    categoryFilter.value = 'all';
    activeSimilarGroupId.value = '';
    openedSimilarGroup.value = null;
    activeSmartBucketId.value = '';
    selection.clearSelection();
    if (backendCommandsAvailable()) {
      reload();
    }
  }

  function setMailboxFilter(next: MailboxFilter) {
    activeContactEmail.value = '';
    activeSimilarGroupId.value = '';
    openedSimilarGroup.value = null;
    activeSmartBucketId.value = '';
    categoryFilter.value = 'all';
    selection.clearSelection();
    mailboxFilter.value = next;
    // Do not leave a detail pane from the previous account/folder visible
    // while its replacement page is in flight. A previously cached row from
    // the destination remains instant; otherwise the pane becomes neutral.
    if (!visibleThreads.value.some((thread) => thread.id === selection.getActiveThreadId())) {
      selection.setActiveThreadId(visibleThreads.value[0]?.id ?? '');
    }
    if (backendCommandsAvailable()) {
      reload();
    }
  }

  let watchersStarted = false;
  function startWatching() {
    if (watchersStarted) return;
    watchersStarted = true;

    watch(activeSimilarGroup, (nextGroup) => {
      if (!nextGroup) {
        if (activeSimilarGroupId.value) {
          activeSimilarGroupId.value = '';
          openedSimilarGroup.value = null;
          selection.clearSelection();
        }
        return;
      }

      if (!nextGroup.threads.some((thread) => thread.id === selection.getActiveThreadId())) {
        selection.setActiveThreadId(nextGroup.threads[0]?.id ?? '');
      }
    });

    // Guarded in the source like the contact watcher below: watching the
    // computed itself kept the bucket projection evaluated in every view.
    watch(
      () => (activeSmartBucketId.value ? smartInboxBuckets.value : null),
      (nextBuckets, previousBuckets) => {
        if (!nextBuckets) {
          return;
        }

        const nextBucket = nextBuckets.find((bucket) => bucket.id === activeSmartBucketId.value);
        if (nextBucket?.threads.length) {
          return;
        }

        // Server summaries can advertise a bucket before its first page has been
        // loaded. Do not close that initially empty shell; only close once a bucket
        // that actually showed threads has lost its final local member.
        const previousBucket = previousBuckets?.find(
          (bucket) => bucket.id === activeSmartBucketId.value,
        );
        if (nextBucket && !previousBucket?.threads.length) {
          return;
        }

        // Reload through the regular close path: the rows on screen are the
        // bucket-filtered page, and leaving the bucket without refetching would
        // leave the Inbox showing that narrow slice instead of its own list.
        closeSmartBucket();
      },
    );

    watch(activeSmartBucket, (nextBucket) => {
      if (!nextBucket) {
        return;
      }

      if (!nextBucket.threads.some((thread) => thread.id === selection.getActiveThreadId())) {
        selection.setActiveThreadId(nextBucket.threads[0]?.id ?? '');
      }
    });

    // The guard sits in the source, not in the callback: watching the computed
    // itself would evaluate the contact projection on every thread mutation,
    // even with no contact filter in sight.
    watch(
      () => (activeContactEmail.value ? contactSummaries.value : null),
      (nextContacts) => {
        if (!nextContacts) {
          return;
        }

        if (backendCommandsAvailable() || threadLoadInFlight.value) {
          return;
        }

        if (!nextContacts.some((contact) => contact.email === activeContactEmail.value)) {
          activeContactEmail.value = '';
          selection.clearSelection();
        }
      },
    );
  }

  projection.scopedVisibleThreads = () => scopedVisibleThreads.value;
  projection.visibleThreadOrder = () => visibleThreadOrder.value;

  const surface: ThreadFilterBucketsSurface = {
    activeSimilarGroupId,
    activeSmartBucketId,
    workflowFilter,
    mailboxFilter,
    categoryFilter,
    activeContactEmail,
    activeSimilarGroup,
    activeSmartBucket,
    activeContact,
    visibleThreads,
    contactSummaries,
    frequentContactSummaries,
    allContactSummaries,
    pinnedContactSummaries,
    similarThreadGroups,
    newSenderStack,
    smartInboxBuckets,
    smartInboxTotalCount,
    smartListEntries,
    scopedVisibleThreads,
    listEntries,
    visibleThreadOrder,
    openSimilarGroup,
    closeSimilarGroup,
    openSmartBucket,
    closeSmartBucket,
    openContact,
    setMailboxFilter,
  };

  return {
    surface,
    filter: { ...surface, compareThreads },
    onScopeChange(listener) {
      scopeListeners.push(listener);
    },
    startWatching,
  };
}
