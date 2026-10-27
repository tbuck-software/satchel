<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useNewSenderActions } from '@/composables/useNewSenderActions';
import { useThreadActions } from '@/composables/useThreadActions';
import { useToast } from '@/composables/useToast';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { useSettingsStore } from '@/stores/settings';
import { useListSearchStore } from '@/stores/listSearch';
import { useThreadsStore, type NewSenderDomainSuggestion, type NewSenderGroup } from '@/stores/threads';
import { useTopicsStore } from '@/stores/topics';
import { THREAD_LIST_WIDTH_LIMITS, useUiStore } from '@/stores/ui';
import { useFollowUpsStore } from '@/stores/followUps';
import { formatListTime } from '@/utils/listTime';
import { NEW_SENDER_PREVIEW_LIMIT } from '@/utils/threadList';
import type { WorkflowState } from '@/types/workflow';
import SmartBucketIcon from '@/components/mail/SmartBucketIcon.vue';
import ThreadGroupList from '@/components/mail/ThreadGroupList.vue';
import ListSearchBar from '@/components/mail/ListSearchBar.vue';
import ListFilterMenu from '@/components/mail/ListFilterMenu.vue';
import ThreadPaneSkeleton from '@/components/mail/ThreadPaneSkeleton.vue';
import TopicsFolderCard from '@/components/topics/TopicsFolderCard.vue';
import Avatar from '@/components/common/Avatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import CountBadge from '@/components/ui/CountBadge.vue';
import IconButton from '@/components/ui/IconButton.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import FilterPill from '@/components/ui/FilterPill.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import PaneResizeHandle from '@/components/ui/PaneResizeHandle.vue';
import { registerListKeyboardNavigation } from '@/composables/useListKeyboardNavigation';
import { useI18n } from 'vue-i18n';

// A single-pane layout navigates to a dedicated thread route. Loading and
// mounting the desktop reader behind the CSS-hidden list wasted message I/O,
// HTML parsing and a large renderer bundle on every phone and tablet startup.
const loadThreadView = () => import('@/components/mail/ThreadView.vue');
const ThreadView = defineAsyncComponent({
  loader: loadThreadView,
  loadingComponent: ThreadPaneSkeleton,
  delay: 80,
});
const TopicDetail = defineAsyncComponent(() => import('@/components/topics/TopicDetail.vue'));
const SelectionToolbar = defineAsyncComponent(() => import('@/components/mail/SelectionToolbar.vue'));
// The search machinery loads with the first query, not with the Inbox.
const ListSearchResults = defineAsyncComponent(() => import('@/components/mail/ListSearchResults.vue'));

const router = useRouter();
const { t } = useI18n();
const threads = useThreadsStore();
const accounts = useAccountsStore();
const connection = useConnectionStore();
const settings = useSettingsStore();
const ui = useUiStore();
const followUps = useFollowUpsStore();
const topicsStore = useTopicsStore();
const threadActions = useThreadActions();
const newSenderActions = useNewSenderActions();
const toast = useToast();
const listSearch = useListSearchStore();

watch(
  () => threads.workflowFilter,
  (filter) => {
    if (filter === 'waiting') void followUps.load();
  },
  { immediate: true },
);

// ── Topics folder: a bucket card; opening it swaps the list for the topics view ──
const topicsViewOpen = ref(false);

function openTopicDetail(id: string) {
  topicsStore.openTopicDetail(id);
}

/** Description stand-in when a topic has none, mirroring the mail row's subject line. */
function topicMetaLabel(topic: { item_count: number; note_count: number }): string {
  const items = t('inbox.topics.items', topic.item_count);
  const notes = t('inbox.topics.notes', topic.note_count);
  return `${items} · ${notes}`;
}

function createTopic() {
  const creation = topicsStore.create({ name: t('inbox.topics.newName') });
  topicsViewOpen.value = true;
  topicsStore.openTopicDetail(creation.topic.id);
  void creation.persisted.catch(() => toast.error(t('inbox.topics.createFailed')));
}

/** A conversation opened from inside a topic replaces the topic in the detail pane. */
function openThreadFromTopic(threadId: string) {
  topicsStore.closeTopicDetail();
  threads.selectThread(threadId);
}

// Sidebar navigation (mailbox, account, workflow, contact, smart folder) must
// land in the mail list — never stay stuck inside the topics view or detail.
watch(
  [
    () => threads.mailboxFilter,
    () => accounts.activeAccountId,
    () => threads.workflowFilter,
    () => threads.activeContactEmail,
  ],
  () => {
    topicsViewOpen.value = false;
    topicsStore.closeTopicDetail();
  },
);
const activeId = computed(() => threads.activeThreadId || threads.scopedVisibleThreads[0]?.id);

const activeThreadPosition = computed(() => {
  const index = threads.scopedVisibleThreads.findIndex((thread) => thread.id === activeId.value);
  return index < 0 ? 0 : index + 1;
});

/** The pointer's version of j and k: one step through the list behind the pane. */
function stepThread(delta: number) {
  if (searchActive.value) {
    const ids = listSearch.resultThreadIds;
    const target = ids[ids.indexOf(searchOpenThreadId.value) + delta];
    if (!target || ids.indexOf(searchOpenThreadId.value) < 0) return;
    listSearch.openThreadId = target;
    threads.selectThread(target);
    return;
  }
  const list = threads.scopedVisibleThreads;
  const index = list.findIndex((thread) => thread.id === activeId.value);
  const target = list[index + delta];
  if (index < 0 || !target) return;
  threads.selectThread(target.id);
}
const activeGroup = computed(() => threads.activeSimilarGroup);
const activeBucket = computed(() => threads.activeSmartBucket);
const activeBundle = computed(() => activeGroup.value ?? activeBucket.value);
const contactViewActive = computed(() => Boolean(threads.activeContact));
const folderViewActive = computed(() => threads.mailboxFilter.kind === 'folder');
const smartInboxActive = computed(() =>
  !folderViewActive.value
    && !contactViewActive.value
    && (settings.inboxGroupingMode === 'smart' || Boolean(activeBucket.value)),
);
const listItems = computed(() =>
  smartInboxActive.value ? threads.smartListEntries : threads.listEntries,
);
// The topics folder lives inside the smart folder section when there is one;
// otherwise (classic list, empty smart inbox) it keeps its spot above the list.
const smartFolderRowVisible = computed(() =>
  listItems.value.some((item) => item.kind === 'folder-row'),
);

type KeyboardListTargetKind = 'thread' | 'bucket' | 'group' | 'topics' | 'new-senders' | 'new-sender';
interface KeyboardListTarget {
  key: string;
  kind: KeyboardListTargetKind;
  id: string;
}

const keyboardFocusKey = ref('');
const keyboardNavigationActive = ref(false);
let pendingKeyboardFocusRestore = '';

/**
 * The strong list focus is a keyboard-navigation aid, not a persistent
 * selection marker. Pointer movement is intentional enough to switch input
 * modality, while pointerdown also covers touchscreens that never hover.
 */
function dismissKeyboardNavigation() {
  if (!keyboardNavigationActive.value) return;
  keyboardNavigationActive.value = false;

  const focused = document.activeElement as HTMLElement | null;
  if (focused?.matches('[data-list-target-key]')) focused.blur();
}

onMounted(() => {
  window.addEventListener('pointerdown', dismissKeyboardNavigation, true);
  window.addEventListener('pointermove', dismissKeyboardNavigation, { passive: true });
});

onBeforeUnmount(() => {
  window.removeEventListener('pointerdown', dismissKeyboardNavigation, true);
  window.removeEventListener('pointermove', dismissKeyboardNavigation);
});

const keyboardTargets = computed<KeyboardListTarget[]>(() => {
  const targets: KeyboardListTarget[] = [];

  if (!activeBundle.value && !smartFolderRowVisible.value) {
    targets.push({ key: 'topics', kind: 'topics', id: 'topics' });
  }

  for (const item of listItems.value) {
    if (item.kind === 'folder-row') {
      if (topicsStore.activeTopics.length > 0) {
        targets.push({ key: 'topics', kind: 'topics', id: 'topics' });
      }
      targets.push(
        ...item.folders.map((folder) => ({
          key: `bucket:${folder.id}`,
          kind: 'bucket' as const,
          id: folder.id,
        })),
      );
    } else if (item.kind === 'priority-card') {
      targets.push(
        ...item.threads.map((entry) => ({
          key: `thread:${entry.id}`,
          kind: 'thread' as const,
          id: entry.id,
        })),
      );
      if (item.hiddenCount > 0) {
        targets.push({ key: `bucket:${item.bucket.id}`, kind: 'bucket', id: item.bucket.id });
      }
    } else if (item.kind === 'bucket') {
      targets.push({ key: `bucket:${item.bucket.id}`, kind: 'bucket', id: item.bucket.id });
    } else if (item.kind === 'group') {
      targets.push({ key: `group:${item.group.id}`, kind: 'group', id: item.group.id });
    } else if (item.kind === 'new-sender-stack') {
      // The banner first; its sender rows only while it is expanded, and the
      // same three the list renders.
      targets.push({ key: `new-senders:${item.stack.id}`, kind: 'new-senders', id: item.stack.id });
      if (ui.newSendersExpanded) {
        targets.push(
          ...item.stack.senders.slice(0, NEW_SENDER_PREVIEW_LIMIT).map((sender) => ({
            key: `new-sender:${sender.id}`,
            kind: 'new-sender' as const,
            id: sender.topThreadId,
          })),
        );
      }
    } else if (item.kind === 'thread') {
      targets.push({ key: `thread:${item.thread.id}`, kind: 'thread', id: item.thread.id });
    }
  }

  return targets.filter((target) => target.id);
});

watch(
  keyboardTargets,
  (targets) => {
    if (pendingKeyboardFocusRestore && targets.some((target) => target.key === pendingKeyboardFocusRestore)) {
      keyboardFocusKey.value = pendingKeyboardFocusRestore;
      pendingKeyboardFocusRestore = '';
      return;
    }
    if (targets.some((target) => target.key === keyboardFocusKey.value)) return;

    const activeMailKey = `thread:${threads.activeThreadId}`;
    keyboardFocusKey.value = targets.some((target) => target.key === activeMailKey)
      ? activeMailKey
      : targets[0]?.key ?? '';
  },
  { immediate: true },
);

watch(
  [keyboardFocusKey, keyboardNavigationActive],
  async ([key, active]) => {
    if (!active || key !== 'topics' || smartFolderRowVisible.value) return;
    await nextTick();
    document.querySelector<HTMLElement>('.topics-folder [data-list-target-key="topics"]')
      ?.focus({ preventScroll: true });
  },
  { flush: 'post' },
);
const listCount = computed(() => {
  if (activeBucket.value) return activeBucket.value.count;
  if (smartInboxActive.value) return threads.smartInboxTotalCount;
  return threads.visibleThreads.length;
});

const categoryCounts = computed(() => {
  const buckets = { all: 0, personal: 0, updates: 0, promotions: 0, social: 0 };
  for (const t of threads.threads) {
    if (!accounts.activeAccountId && !accounts.isVisible(t.accountId)) continue;
    if (accounts.activeAccountId && t.accountId !== accounts.activeAccountId) continue;
    if (threads.mailboxFilter.kind !== 'section' || threads.mailboxFilter.value !== 'inbox') continue;
    if (t.workflowState !== 'inbox') continue;
    buckets.all += 1;
    const cat = t.category ?? 'personal';
    if (cat in buckets) {
      buckets[cat as keyof typeof buckets] += 1;
    }
  }
  return buckets;
});

const categoryTabs = computed(() => [
  { value: 'all' as const, label: t('common.all'), count: categoryCounts.value.all },
  { value: 'personal' as const, label: t('inbox.category.personal'), count: categoryCounts.value.personal },
  { value: 'updates' as const, label: t('inbox.category.updates'), count: categoryCounts.value.updates },
  { value: 'promotions' as const, label: t('inbox.category.promotions'), count: categoryCounts.value.promotions },
  { value: 'social' as const, label: t('inbox.category.social'), count: categoryCounts.value.social },
]);

const categoryFilterVisible = computed(
  () => !contactViewActive.value
    && !smartInboxActive.value
    && threads.mailboxFilter.kind === 'section'
    && threads.mailboxFilter.value === 'inbox',
);

const bulkBusy = ref(false);

const mailboxSectionLabelKeys: Record<string, string> = {
  inbox: 'folders.inbox',
  drafts: 'folders.drafts',
  sent: 'folders.sent',
  trash: 'folders.trash',
  archive: 'folders.archive',
  spam: 'folders.spam',
  notifications: 'folders.notifications',
  blocked: 'folders.blocked',
};

const listTitle = computed(() => {
  if (threads.activeContact) {
    return threads.activeContact.name;
  }

  if (threads.mailboxFilter.kind === 'folder' && threads.mailboxFilter.value) {
    return threads.mailboxFilter.value;
  }

  if (threads.mailboxFilter.kind === 'section' && threads.mailboxFilter.value) {
    const key = mailboxSectionLabelKeys[threads.mailboxFilter.value];
    return key ? t(key) : threads.mailboxFilter.value;
  }

  if (!accounts.activeAccountId && accounts.accounts.length > 1) {
    if (threads.workflowFilter === 'all') return t('folders.allAccounts');
  }
  const workflowLabelKeys: Record<string, string> = {
    inbox: 'folders.inbox',
    todo: 'folders.todo',
    waiting: 'folders.waitingForReply',
    aside: 'folders.aside',
    done: 'thread.state.done',
    snoozed: 'folders.snoozedLong',
  };
  const filter = threads.workflowFilter;
  if (filter === 'all') return t('folders.allMail');
  const key = workflowLabelKeys[filter];
  return key ? t(key) : filter;
});

const workflowEmpty = computed(() => {
  if (activeBundle.value || threads.activeContact || threads.mailboxFilter.kind !== 'all') return null;
  switch (threads.workflowFilter) {
    case 'waiting': return { title: t('inbox.empty.waiting.title'), description: t('inbox.empty.waiting.description'), icon: 'waiting' };
    case 'snoozed': return { title: t('inbox.empty.snoozed.title'), description: t('inbox.empty.snoozed.description'), icon: 'clock' };
    case 'aside': return { title: t('inbox.empty.aside.title'), description: t('inbox.empty.aside.description'), icon: 'aside' };
    default: return null;
  }
});

const listScope = computed(() => {
  if (threads.activeContact) {
    return `${threads.activeContact.email} · ${t('inbox.conversationCount', threads.activeContact.threadCount)}`;
  }

  return accounts.accountScopeLabel;
});

// ── List search: the field above the list narrows it in place ──
const searchActive = computed(() => listSearch.active && !activeBundle.value);
const searchMailbox = computed(() => ({
  accountId: accounts.activeAccountId,
  workflowFilter: threads.workflowFilter,
  mailboxFilter: threads.mailboxFilter,
  activeContactEmail: threads.activeContactEmail,
}));
/** The result in the reading pane: the one opened, else the first hit. */
const searchOpenThreadId = computed(() => {
  const ids = listSearch.resultThreadIds;
  if (listSearch.openThreadId && ids.includes(listSearch.openThreadId)) return listSearch.openThreadId;
  return ui.isSinglePane ? '' : ids[0] ?? '';
});
watch(searchOpenThreadId, (id) => {
  if (id && searchActive.value) void threads.ensureThreadLoaded(id, { retainAcrossListLoads: true });
});
const detailThreadId = computed(() => (searchActive.value ? searchOpenThreadId.value : activeId.value));
const detailPosition = computed(() =>
  searchActive.value ? listSearch.resultThreadIds.indexOf(searchOpenThreadId.value) + 1 : activeThreadPosition.value,
);
const detailTotal = computed(() =>
  searchActive.value ? listSearch.resultThreadIds.length : threads.scopedVisibleThreads.length,
);

// Esc gives the list back as it was: the mailbox list stays mounted under
// the results (scroll position included), and the open thread returns.
let threadBeforeSearch = '';
watch(() => listSearch.active, (active) => {
  if (active) {
    threadBeforeSearch = threads.activeThreadId;
    return;
  }
  if (threadBeforeSearch && threads.scopedVisibleThreads.some((thread) => thread.id === threadBeforeSearch)) {
    threads.selectThread(threadBeforeSearch);
  }
  threadBeforeSearch = '';
});

// Picking another list in the sidebar means "show me that list".
watch(
  [() => threads.mailboxFilter, () => threads.workflowFilter, () => threads.activeContactEmail],
  () => listSearch.clear(),
);

const bundleTitle = computed(() => {
  if (activeGroup.value) {
    return activeGroup.value.ciProject
      ? t('thread.bundle.ciProject', { project: activeGroup.value.ciProject })
      : activeGroup.value.senderName;
  }
  return activeBucket.value?.label ?? '';
});

const bundleSubtitle = computed(() => {
  if (activeGroup.value) {
    return activeGroup.value.title;
  }
  return t('inbox.smartFolder');
});

function onActivate(payload: { threadId: string; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean; keyboard?: boolean }) {
  keyboardFocusKey.value = `thread:${payload.threadId}`;
  keyboardNavigationActive.value = payload.keyboard ?? false;
  topicsStore.closeTopicDetail();
  threads.selectFromInteraction(
    payload.threadId,
    {
      shiftKey: payload.shiftKey,
      metaKey: payload.metaKey,
      ctrlKey: payload.ctrlKey,
    },
    true,
  );

  if (ui.isSinglePane || payload.keyboard) {
    void router.push({ name: 'thread', params: { id: payload.threadId } });
  }
}

function onCheck(payload: { threadId: string; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }) {
  threads.selectFromInteraction(
    payload.threadId,
    {
      shiftKey: payload.shiftKey,
      metaKey: payload.metaKey,
      ctrlKey: payload.ctrlKey,
    },
    false,
  );
}

async function runStateForThreads(threadIds: string[], nextState: WorkflowState, label: string) {
  if (bulkBusy.value) {
    return;
  }
  bulkBusy.value = true;
  try {
    // Shared with the keyboard path so both offer the same undo affordance.
    await threadActions.transitionThreads(threadIds, nextState, label);
  } finally {
    bulkBusy.value = false;
  }
}

async function runActiveGroupDone() {
  const threadIds = activeBundle.value?.threads.map((thread) => thread.id) ?? [];
  await runStateForThreads(threadIds, 'done', t('common.done'));
  // Checking off the last member already clears the group through the store's
  // own watcher, which returns to the enclosing list in the same beat. Calling
  // closeGroup() here would find no group left and fall through to its
  // "close the bucket" fallback — a second exit, one or two seconds later
  // once persistence settles, yanking the user out of the folder they are
  // still reading. Close only what is actually still open.
  if (activeGroup.value) {
    threads.closeSimilarGroup();
  }
}

function openGroup(groupId: string) {
  threads.openSimilarGroup(groupId);
}

function openBucket(bucketId: string) {
  threads.openSmartBucket(bucketId);
}

function openNewSenders() {
  void router.push({ name: 'new-senders' });
}

function closeGroup() {
  if (activeGroup.value) {
    threads.closeSimilarGroup();
    return;
  }

  threads.closeSmartBucket();
}

function moveKeyboardFocus(delta: 1 | -1, extendSelection = false): boolean {
  const targets = keyboardTargets.value;
  if (targets.length === 0) return false;

  const activeMailKey = `thread:${threads.activeThreadId}`;
  const currentKey = keyboardNavigationActive.value
    ? keyboardFocusKey.value
    : targets.some((target) => target.key === activeMailKey)
      ? activeMailKey
      : keyboardFocusKey.value;
  const currentIndex = Math.max(0, targets.findIndex((target) => target.key === currentKey));
  const currentTarget = targets[currentIndex];
  if (!currentTarget) return false;
  const nextIndex = Math.min(targets.length - 1, Math.max(0, currentIndex + delta));
  const target = targets[nextIndex];
  if (!target) return false;
  keyboardFocusKey.value = target.key;
  keyboardNavigationActive.value = true;

  if ((target.kind === 'thread' || target.kind === 'new-sender') && target.id) {
    if (extendSelection) {
      if (
        threads.selectedThreadIds.length === 0
        && (currentTarget.kind === 'thread' || currentTarget.kind === 'new-sender')
      ) {
        threads.selectFromInteraction(
          currentTarget.id,
          { shiftKey: false, metaKey: false, ctrlKey: false },
          false,
        );
      }
      threads.selectFromInteraction(
        target.id,
        { shiftKey: true, metaKey: false, ctrlKey: false },
        true,
      );
    } else {
      threads.selectThread(target.id);
    }
  }
  return true;
}

function openKeyboardFocus(): boolean {
  const target = keyboardTargets.value.find((item) => item.key === keyboardFocusKey.value);
  if (!target) return false;

  keyboardNavigationActive.value = true;
  if (target.kind === 'topics') {
    topicsViewOpen.value = true;
    return true;
  }
  if (target.kind === 'bucket') {
    pendingKeyboardFocusRestore = target.key;
    openBucket(target.id);
    return true;
  }
  if (target.kind === 'group') {
    pendingKeyboardFocusRestore = target.key;
    openGroup(target.id);
    return true;
  }
  if (target.kind === 'new-senders') {
    ui.toggleNewSendersExpanded();
    return true;
  }

  if (target.id) {
    threads.selectThread(target.id);
    void router.push({ name: 'thread', params: { id: target.id } });
    return true;
  }
  return false;
}

function leaveKeyboardLevel(): boolean {
  if (topicsStore.activeTopicId) {
    topicsStore.closeTopicDetail();
    return true;
  }
  if (topicsViewOpen.value) {
    topicsViewOpen.value = false;
    topicsStore.closeTopicDetail();
    pendingKeyboardFocusRestore = 'topics';
    return true;
  }
  if (activeGroup.value) {
    pendingKeyboardFocusRestore = `group:${activeGroup.value.id}`;
    closeGroup();
    return true;
  }
  if (activeBucket.value) {
    pendingKeyboardFocusRestore = `bucket:${activeBucket.value.id}`;
    closeGroup();
    return true;
  }
  return false;
}

async function focusRestoredThread(threadId: string): Promise<boolean> {
  const key = `thread:${threadId}`;
  if (!keyboardTargets.value.some(target => target.key === key)) {
    // A smart-folder action can close its folder when the last member leaves.
    const bucket = threads.smartInboxBuckets.find(entry => entry.threads.some(thread => thread.id === threadId));
    if (bucket) threads.openSmartBucket(bucket.id);
    const group = threads.similarThreadGroups.find(entry => entry.threads.some(thread => thread.id === threadId));
    if (group) threads.openSimilarGroup(group.id);
    await nextTick();
  }
  if (!keyboardTargets.value.some(target => target.key === key)) return false;
  keyboardNavigationActive.value = false;
  await nextTick();
  keyboardFocusKey.value = key;
  keyboardNavigationActive.value = true;
  await nextTick();
  return true;
}

onBeforeUnmount(registerListKeyboardNavigation({
  move: moveKeyboardFocus,
  open: openKeyboardFocus,
  back: leaveKeyboardLevel,
  focusThread: focusRestoredThread,
}));

// These screens are store state rather than routes. Register each level with
// the shared Back stack so Android's edge gesture follows the same hierarchy
// as the visible back buttons. Dialogs and the mobile drawer remain above
// these entries and are dismissed first by the ui store.
onBeforeUnmount(ui.registerBackNavigation({
  isOpen: () =>
    threads.workflowFilter !== 'all'
    || threads.mailboxFilter.kind !== 'section'
    || threads.mailboxFilter.value !== 'inbox'
    || Boolean(threads.activeContactEmail),
  dismiss: () => {
    threads.workflowFilter = 'all';
    threads.setMailboxFilter({ kind: 'section', value: 'inbox' });
  },
}));
onBeforeUnmount(ui.registerBackNavigation({
  isOpen: () => Boolean(activeBundle.value),
  dismiss: () => { void leaveKeyboardLevel(); },
}));
onBeforeUnmount(ui.registerBackNavigation({
  isOpen: () => topicsViewOpen.value,
  dismiss: () => { void leaveKeyboardLevel(); },
}));
onBeforeUnmount(ui.registerBackNavigation({
  isOpen: () => ui.isSinglePane && Boolean(topicsStore.activeTopicId),
  dismiss: () => topicsStore.closeTopicDetail(),
}));

async function onBucketAction(payload: { threadIds: string[]; action: 'done' | 'read' }) {
  if (payload.action === 'done') {
    await runStateForThreads(payload.threadIds, 'done', t('common.done'));
    return;
  }

  if (!await threadActions.setRead(payload.threadIds, true)) return;
  toast.success(t('inbox.markedReadToast', payload.threadIds.length));
}

async function onGroupDone(threadIds: string[]) {
  await runStateForThreads(threadIds, 'done', t('common.done'));
}

async function onDateGroupDone(payload: { groupId: string; threadIds: string[] }) {
  // Header payloads are render-time snapshots. Re-intersect them with the
  // currently visible date groups so a thread pinned or set aside just before
  // the click cannot be filed by a stale month action.
  const currentGroup = listItems.value.find(
    (entry) => entry.kind === 'date-header' && entry.id === payload.groupId,
  );
  if (!currentGroup || currentGroup.kind !== 'date-header') return;
  const currentDateThreadIds = new Set(currentGroup.threadIds);
  const visibleIds = payload.threadIds.filter((id) => currentDateThreadIds.has(id));
  if (visibleIds.length === 0) return;
  await runStateForThreads(visibleIds, 'done', t('common.done'));
}

async function onNewSenderAction(payload: { sender: NewSenderGroup; action: 'accept' | 'block' }) {
  if (bulkBusy.value) {
    return;
  }

  if (payload.action === 'accept') {
    newSenderActions.acceptSender(payload.sender);
    return;
  }

  bulkBusy.value = true;
  try {
    await newSenderActions.blockSender(
      payload.sender,
      t('newSenders.blockConfirm', { email: payload.sender.senderEmail }),
    );
  } finally {
    bulkBusy.value = false;
  }
}

function onAcceptDomain(suggestion: NewSenderDomainSuggestion) {
  settings.acceptDomain(suggestion.domain);
  toast.success(t('newSenders.domainAllowedToast', { domain: suggestion.domain }));
}

function acceptExistingSenders() {
  const stack = threads.newSenderStack;
  if (!stack) return;
  settings.completeSenderReviewSetup(stack.senders.map((sender) => sender.senderEmail));
}
</script>

<template>
  <section class="inbox-layout" :class="{ 'inbox-layout--single': ui.isSinglePane }" :style="{ '--list-w': `${ui.threadListWidth}px` }">
    <!-- Thread List Pane -->
    <article class="list-pane min-h-0 grid grid-cols-[minmax(0,1fr)] grid-rows-[auto_auto_auto_auto_minmax(0,1fr)] overflow-hidden bg-list border-r border-border-muted">
      <!-- Topics view: inside the folder, topics listed like mails -->
      <template v-if="topicsViewOpen">
        <PaneHeader :title="$t('inbox.topics.title')" :count="topicsStore.activeTopics.length">
          <template #leading>
            <IconButton :title="$t('common.back')" @click="topicsViewOpen = false">
              <svg width="18" height="18" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <path d="M10 3.5L5 8l5 4.5" />
              </svg>
            </IconButton>
          </template>
          <template #actions>
            <IconButton :title="$t('inbox.topics.new')" @click="void createTopic()">
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                <path d="M8 3V13M3 8H13" />
              </svg>
            </IconButton>
          </template>
        </PaneHeader>

        <div class="topics-view row-start-4">
          <!-- Rows share the mail rows' visual language: same avatar footprint,
               unread dot, type scale and compact time. A topic is read like a mail. -->
          <CardButton
            v-for="topic in topicsStore.activeTopics"
            :key="topic.id"
            class="group relative mb-1 grid h-[72px] w-full cursor-pointer select-none grid-cols-[auto_minmax(0,1fr)] items-center gap-3 overflow-hidden rounded-xl border-0 px-3 text-left transition-colors"
            :class="
              topicsStore.activeTopicId === topic.id
                ? 'bg-surface shadow-sm'
                : 'bg-transparent hover:bg-hover'
            "
            @click="openTopicDetail(topic.id)"
          >
            <!-- Staged like the folder chips: icon, else newest sender, else initials. -->
            <span v-if="topic.icon" class="topic-avatar-emoji" aria-hidden="true">{{ topic.icon }}</span>
            <Avatar
              v-else
              :name="topic.last_sender_name || topic.last_sender_email || topic.name"
              :email="topic.last_sender_email"
              :size="38"
              aria-hidden="true"
            />
            <span class="flex min-h-0 min-w-0 flex-col justify-center">
              <span class="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-start gap-2">
                <span class="flex min-w-0 items-center gap-1.5">
                  <span
                    v-if="topic.unread_count > 0"
                    class="h-1.5 w-1.5 shrink-0 rounded-full bg-accent-primary"
                    aria-hidden="true"
                  />
                  <span
                    class="block min-w-0 truncate text-md transition-colors"
                    :class="
                      topicsStore.activeTopicId === topic.id || topic.unread_count > 0
                        ? 'font-strong text-primary'
                        : 'font-medium text-secondary'
                    "
                  >
                    {{ topic.name }}
                  </span>
                </span>
                <span class="whitespace-nowrap pt-0.5 text-right text-xs leading-[1.4] text-tertiary tabular-nums">
                  {{ formatListTime(topic.updated_at) }}
                </span>
              </span>
              <span class="mt-0.5 flex min-w-0 items-center gap-[7px]">
                <span
                  class="min-w-0 truncate text-sm"
                  :class="
                    topicsStore.activeTopicId === topic.id || topic.unread_count > 0
                      ? 'font-medium text-[color-mix(in_srgb,var(--text-primary)_78%,var(--text-secondary))]'
                      : 'font-medium text-[color-mix(in_srgb,var(--text-secondary)_82%,var(--text-tertiary))]'
                  "
                >
                  {{ topic.description || topicMetaLabel(topic) }}
                </span>
                <span
                  v-if="topic.due_date && topic.due_date <= new Date().toISOString()"
                  class="topics-folder__due"
                >{{ $t('inbox.topics.due') }}</span>
                <CountBadge v-if="topic.item_count > 1" :value="topic.item_count" />
              </span>
            </span>
          </CardButton>

          <p v-if="topicsStore.activeTopics.length === 0" class="topics-view__empty">
            {{ $t('inbox.topics.empty') }}
          </p>

          <template v-if="topicsStore.finishedTopics.length > 0">
            <SectionHeading :label="$t('common.done')" :count="topicsStore.finishedTopics.length" class="topics-view__heading" />
            <CardButton
              v-for="topic in topicsStore.finishedTopics"
              :key="topic.id"
              class="group relative mb-1 grid h-[56px] w-full cursor-pointer select-none grid-cols-[auto_minmax(0,1fr)] items-center gap-3 overflow-hidden rounded-xl border-0 px-3 text-left opacity-75 transition-colors"
              :class="
                topicsStore.activeTopicId === topic.id
                  ? 'bg-surface shadow-sm'
                  : 'bg-transparent hover:bg-hover'
              "
              @click="openTopicDetail(topic.id)"
            >
              <span class="topic-avatar-done" aria-hidden="true">
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M2.5 8.5L6.5 12.5L13.5 4" />
                </svg>
              </span>
              <span class="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-2">
                <span class="block min-w-0 truncate text-md font-medium text-secondary">{{ topic.name }}</span>
                <span class="whitespace-nowrap text-right text-xs leading-[1.4] text-tertiary tabular-nums">
                  {{ formatListTime(topic.updated_at) }}
                </span>
              </span>
            </CardButton>
          </template>
        </div>
      </template>

      <template v-else>
      <PaneHeader
        v-if="!activeBundle"
        class="inbox-header"
        :title="listTitle"
        :subtitle="listScope"
        :count="listCount"
      >
      </PaneHeader>

      <PaneHeader
        v-else
        :title="bundleTitle"
        :subtitle="bundleSubtitle"
        :count="activeBundle?.count"
        class="bundle-header"
      >
        <template #leading>
          <IconButton :title="$t('common.back')" @click="closeGroup">
            <svg width="18" height="18" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M10 3.5L5 8l5 4.5" />
            </svg>
          </IconButton>
          <SmartBucketIcon
            v-if="activeBucket"
            :bucket-key="activeBucket.key"
            :tone="activeBucket.tone"
            :size="34"
          />
          <Avatar
            v-else-if="activeGroup"
            :name="activeGroup.senderName"
            :email="activeGroup.senderEmail"
            :size="34"
          />
        </template>
        <template #actions>
          <AppButton size="sm" :disabled="bulkBusy" :title="$t('inbox.bundleDoneTitle')" @click="void runActiveGroupDone()">
            <template #icon>
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                <path d="M3.5 8.4l2.5 2.5 6.5-6.8" />
              </svg>
            </template>
            {{ $t('inbox.bundleDone') }}
          </AppButton>
        </template>
      </PaneHeader>

      <ListSearchBar v-if="!activeBundle" :mailbox-label="listTitle">
        <ListFilterMenu :show-layout="!folderViewActive" />
      </ListSearchBar>

      <!-- How the list is sliced. The account it is drawn from is chosen in
           the filter menu, on phones too: one account control, not two. -->
      <nav
        v-if="!activeBundle && !searchActive && categoryFilterVisible"
        class="filter-strip"
        :aria-label="$t('inbox.filterStrip')"
      >
        <FilterPill
          v-for="tab in categoryTabs"
          :key="tab.value"
          :label="tab.label"
          :count="tab.count"
          :pressed="threads.categoryFilter === tab.value"
          @click="threads.categoryFilter = tab.value"
        />
      </nav>

      <SelectionToolbar v-if="threads.selectedThreadIds.length > 0" :busy="bulkBusy" />

      <!-- Thread list cell: topics folder on top, the virtualised list below.
           Search results cover both; the list stays mounted underneath so
           leaving the search returns to the same scroll position. -->
      <div class="list-cell relative row-start-5 min-h-0 min-w-0 overflow-hidden flex flex-col">
      <ListSearchResults
        v-if="searchActive"
        class="list-search-overlay"
        :mailbox-label="listTitle"
        :mailbox="searchMailbox"
        :open-thread-id="searchOpenThreadId"
      />
      <!-- Topics folder: cross-source collections, opened in the detail pane.
           Only rendered here when the list has no smart folder section to join. -->
      <div v-if="!activeBundle && !smartFolderRowVisible && !(workflowEmpty && listItems.length === 0)" class="topics-folder no-drag shrink-0" :inert="searchActive || undefined">
        <TopicsFolderCard
          :topics="topicsStore.activeTopics"
          target-key="topics"
          :keyboard-focused="keyboardNavigationActive && keyboardFocusKey === 'topics'"
          @open="topicsViewOpen = true"
          @create="void createTopic()"
        />
      </div>

      <!-- Thread List -->
      <div class="min-h-0 min-w-0 flex-1 overflow-hidden" :inert="searchActive || undefined">
        <ThreadGroupList
          :items="listItems"
          :loading="threads.threadLoadInFlight"
          :load-more-failed="threads.loadMoreFailed"
          :load-previous-failed="threads.loadPreviousFailed"
          :has-previous-threads="threads.hasPreviousThreads"
          :pagination-busy="threads.loadingMoreThreads"
          :error="connection.backendErrorFor('threads')"
          :backfill="{ remaining: connection.backfillRemaining, total: connection.backfillTotal }"
          :active-thread-id="ui.isSinglePane ? undefined : activeId"
          :focused-target-key="keyboardFocusKey"
          :keyboard-navigation-active="keyboardNavigationActive"
          :selected-thread-ids="threads.selectedThreadIds"
          :active-group-id="activeBundle?.id"
          :sender-review-setup-complete="Boolean(settings.senderReviewStartedAt)"
          :new-senders-expanded="ui.newSendersExpanded"
          :bulk-busy="bulkBusy"
          :topics="topicsStore.activeTopics"
          @load-more="void threads.loadMore()"
          @load-previous="void threads.loadPrevious()"
          @retry-load-more="void threads.retryLoadMore()"
          @viewport-start="threads.setViewportStart($event)"
          @retry="void threads.load()"
          @activate="onActivate"
          @check="onCheck"
          @open-group="openGroup"
          @open-bucket="openBucket"
          @open-topics="topicsViewOpen = true"
          @create-topic="void createTopic()"
          @open-new-senders="openNewSenders"
          @toggle-new-senders="ui.toggleNewSendersExpanded()"
          @accept-existing-senders="acceptExistingSenders"
          @bucket-action="onBucketAction"
          @group-done="onGroupDone"
          @date-group-done="onDateGroupDone"
          @new-sender-action="onNewSenderAction"
          @accept-domain="onAcceptDomain"
        >
          <template v-if="workflowEmpty" #empty>
            <EmptyState :title="workflowEmpty.title" :description="workflowEmpty.description">
              <template #icon>
                <AppIcon v-if="workflowEmpty.icon === 'clock'" size="lg"><circle cx="8" cy="8" r="6" /><path d="M8 4v4l3 2" /></AppIcon>
                <AppIcon v-else-if="workflowEmpty.icon === 'waiting'" size="lg"><path d="M2 3h12v8H8l-4 3v-3H2zM5 6h6M5 8h3" /></AppIcon>
                <AppIcon v-else size="lg"><rect x="2" y="4" width="12" height="9" rx="1" /><path d="M6 4V2h4v2M2 7h12M6 9h4" /></AppIcon>
              </template>
            </EmptyState>
          </template>
        </ThreadGroupList>
      </div>
      </div>
      </template>
    </article>

    <!-- Resize Handle -->
    <PaneResizeHandle
      v-if="!ui.isSinglePane"
      class="list-resize"
      :label="$t('inbox.resizeList')"
      :value="ui.threadListWidth"
      :min="THREAD_LIST_WIDTH_LIMITS.min"
      :max="THREAD_LIST_WIDTH_LIMITS.max"
      @resize="ui.setThreadListWidth"
    />

    <!-- Detail Pane -->
    <article v-if="!ui.isSinglePane" class="detail-pane min-h-0 overflow-hidden bg-surface">
      <TopicDetail
        v-if="topicsStore.activeTopicId"
        :topic-id="topicsStore.activeTopicId"
        @deleted="topicsStore.closeTopicDetail()"
        @open-thread="openThreadFromTopic"
      />
      <ThreadView
        v-else
        :thread-id="detailThreadId"
        :collapse-to-latest="!searchActive && threads.mailboxFilter.kind === 'section' && threads.mailboxFilter.value === 'drafts'"
        :position="detailPosition"
        :total="detailTotal"
        @previous="stepThread(-1)"
        @next="stepThread(1)"
      />
    </article>

  </section>

  <!-- Undo Toast -->
  <Transition name="slide-up">
  </Transition>
</template>

<style scoped>
.inbox-layout {
  height: 100%;
  display: grid;
  grid-template-columns: var(--list-w) 1px minmax(0, 1fr);
  grid-template-rows: 1fr;
  overflow: hidden;
}

.list-pane {
  grid-row: 1;
}

.detail-pane {
  grid-row: 1;
}

/* ── Topics folder above the thread list, same card as the smart folders ── */
.topics-folder {
  padding: 4px 8px;
  border-bottom: 1px solid var(--border-color-muted);
  background: color-mix(in srgb, var(--bg-surface) 32%, var(--bg-base));
}

/* ── Topics view: topics listed like mails, opened from the folder card ── */
.topics-view {
  min-height: 0;
  overflow-y: auto;
  padding: 4px 8px;
}

/* Emoji stand-in with the mail avatar's exact footprint. */
.topic-avatar-emoji {
  display: grid;
  place-items: center;
  width: 38px;
  height: 38px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-purple) 16%, transparent);
  font-size: 18px;
  line-height: 1;
  flex-shrink: 0;
}

.topic-avatar-done {
  display: grid;
  place-items: center;
  width: 38px;
  height: 38px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-green) 14%, transparent);
  color: var(--accent-green);
  flex-shrink: 0;
}

.topics-view__empty {
  margin: 0;
  padding: 18px 16px;
  font-size: var(--font-size-sm);
  color: var(--text-tertiary);
}

.topics-view__heading {
  padding: 14px 14px 4px;
}

.topics-folder__due {
  flex-shrink: 0;
  padding: 1px 7px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-amber) 16%, transparent);
  color: var(--accent-amber);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-bold);
}

.list-resize {
  grid-row: 1;
}

@media (max-width: 1020px) {
  .inbox-header :deep(.pane-header__actions) {
    align-self: flex-start;
  }
}

.bundle-header {
  border-bottom: 1px solid var(--border-color-muted);
}

.inbox-layout--single {
  grid-template-columns: minmax(0, 1fr);
}

/* Opaque and on top; the list underneath is only made inert. Toggling its
   visibility left WebKit with unpainted rows when the search closed. */
.list-search-overlay {
  position: absolute;
  inset: 0;
  z-index: 2;
}

.filter-strip {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
  padding: 0 var(--space-lg) var(--space-2-5);
  background: var(--bg-list);
  overflow-x: auto;
  scrollbar-width: none;
}

.filter-strip::-webkit-scrollbar {
  display: none;
}

.category-tab.active .count {
  background: color-mix(in srgb, var(--accent-primary) 18%, transparent);
  color: var(--accent-primary);
}

</style>
