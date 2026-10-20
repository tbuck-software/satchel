<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useNewSenderActions } from '@/composables/useNewSenderActions';
import { useThreadActions } from '@/composables/useThreadActions';
import { useToast } from '@/composables/useToast';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { useSettingsStore, type InboxGroupingMode, type InboxSenderBucket } from '@/stores/settings';
import { useThreadsStore, type NewSenderDomainSuggestion, type NewSenderGroup } from '@/stores/threads';
import { useTopicsStore } from '@/stores/topics';
import { THREAD_LIST_WIDTH_LIMITS, useUiStore } from '@/stores/ui';
import { useFollowUpsStore } from '@/stores/followUps';
import { formatListTime } from '@/utils/listTime';
import type { ThreadCategory } from '@/types/mail';
import type { WorkflowState } from '@/types/workflow';
import SmartBucketIcon from '@/components/mail/SmartBucketIcon.vue';
import ThreadGroupList from '@/components/mail/ThreadGroupList.vue';
import ThreadPaneSkeleton from '@/components/mail/ThreadPaneSkeleton.vue';
import TopicsFolderCard from '@/components/topics/TopicsFolderCard.vue';
import Avatar from '@/components/common/Avatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import CountBadge from '@/components/ui/CountBadge.vue';
import IconButton from '@/components/ui/IconButton.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import FilterPill from '@/components/ui/FilterPill.vue';
import SearchField from '@/components/ui/SearchField.vue';
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
const SnoozePicker = defineAsyncComponent(() => import('@/components/navigation/SnoozePicker.vue'));

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

type KeyboardListTargetKind = 'thread' | 'bucket' | 'group' | 'topics' | 'new-sender';
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
      targets.push({
        key: `new-sender:${item.stack.id}`,
        kind: 'new-sender',
        id: item.stack.senders[0]?.topThreadId ?? '',
      });
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
const selectedCategory = ref<ThreadCategory>('updates');
const rememberSelectedSenders = ref(false);

const categoryCounts = computed(() => {
  const buckets = { all: 0, personal: 0, updates: 0, promotions: 0, social: 0 };
  for (const t of threads.threads) {
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

const accountFilterVisible = computed(() => ui.isMobile && accounts.accounts.length > 1);

const categoryFilterVisible = computed(
  () => !contactViewActive.value
    && !smartInboxActive.value
    && threads.mailboxFilter.kind === 'section'
    && threads.mailboxFilter.value === 'inbox',
);

const classificationOptions = computed<Array<{
  label: string;
  category: ThreadCategory;
  senderBucket: InboxSenderBucket;
}>>(() => [
  { label: t('thread.bucket.personal'), category: 'personal', senderBucket: 'personal' },
  { label: t('thread.bucket.notifications'), category: 'updates', senderBucket: 'notifications' },
  { label: t('thread.bucket.newsletter'), category: 'promotions', senderBucket: 'newsletter' },
]);

const selectedCountLabel = computed(() => {
  const count = threads.selectedThreadIds.length;
  return t('inbox.mailCount', count);
});
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

const listScope = computed(() => {
  if (threads.activeContact) {
    return `${threads.activeContact.email} · ${t('inbox.conversationCount', threads.activeContact.threadCount)}`;
  }

  return accounts.accountScopeLabel;
});

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

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onViewMenuPointerDown);
  document.removeEventListener('keydown', onViewMenuKeydown);
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

const viewMenuOpen = ref(false);
const viewMenuRef = ref<HTMLElement | null>(null);

function toggleViewMenu() {
  viewMenuOpen.value = !viewMenuOpen.value;
}

function closeViewMenu() {
  viewMenuOpen.value = false;
}

function onViewMenuPointerDown(event: PointerEvent) {
  if (!viewMenuRef.value?.contains(event.target as Node)) closeViewMenu();
}

function onViewMenuKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') closeViewMenu();
}

watch(viewMenuOpen, (open) => {
  if (open) {
    document.addEventListener('pointerdown', onViewMenuPointerDown);
    document.addEventListener('keydown', onViewMenuKeydown);
  } else {
    document.removeEventListener('pointerdown', onViewMenuPointerDown);
    document.removeEventListener('keydown', onViewMenuKeydown);
  }
});

function chooseGroupingMode(mode: InboxGroupingMode) {
  settings.setInboxGroupingMode(mode);
  if (mode === 'classic') {
    threads.closeSmartBucket();
  }
  if (mode === 'smart') {
    threads.categoryFilter = 'all';
  }
  closeViewMenu();
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

onBeforeUnmount(registerListKeyboardNavigation({
  move: moveKeyboardFocus,
  open: openKeyboardFocus,
  back: leaveKeyboardLevel,
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

async function classifySelected() {
  const option = classificationOptions.value.find((entry) => entry.category === selectedCategory.value);
  if (!option || threads.selectedThreadIds.length === 0 || bulkBusy.value) {
    return;
  }

  const selectedThreads = threads.selectedThreads;
  bulkBusy.value = true;
  const applied = await threadActions.attempt(async () => {
    // Wait for every write, including failures, before releasing the selection.
    const outcomes = await Promise.allSettled(
      selectedThreads.map((thread) => rememberSelectedSenders.value
        ? threads.setSenderCategory(thread.id, option.category, option.senderBucket)
        : threads.setThreadCategory(thread.id, option.category)),
    );
    const failed = outcomes.find((outcome) => outcome.status === 'rejected');
    if (failed?.status === 'rejected') throw failed.reason;
  });
  bulkBusy.value = false;
  if (!applied) return;
  threads.clearSelection();
  toast.success(t('inbox.classifiedToast', selectedThreads.length));
}

const bulkSnoozePickerOpen = ref(false);
const bulkSnoozeIds = ref<string[]>([]);

function openBulkSnooze() {
  bulkSnoozeIds.value = [...threads.selectedThreadIds];
  bulkSnoozePickerOpen.value = bulkSnoozeIds.value.length > 0;
}

async function applyBulkSnooze(date: string) {
  bulkSnoozePickerOpen.value = false;
  await threadActions.snoozeAt(bulkSnoozeIds.value, date);
  bulkSnoozeIds.value = [];
}

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
    <article class="list-pane min-h-0 grid grid-cols-[minmax(0,1fr)] grid-rows-[auto_auto_auto_auto_1fr] overflow-hidden bg-list border-r border-border-muted">
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
        :title="listTitle"
        :subtitle="listScope"
        :count="listCount"
      >
        <template #actions>
          <div v-if="!folderViewActive" ref="viewMenuRef" class="view-menu-wrap">
            <IconButton
              :title="$t('inbox.view')"
              :active="viewMenuOpen"
              aria-haspopup="menu"
              :aria-expanded="viewMenuOpen"
              @click="toggleViewMenu"
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                <path d="M3 4.5h10" />
                <path d="M4.75 8h6.5" />
                <path d="M6.5 11.5h3" />
              </svg>
            </IconButton>
            <Transition name="menu-pop">
              <div v-if="viewMenuOpen" class="view-menu" role="menu" :aria-label="$t('inbox.view')">
                <p class="view-menu-label">{{ $t('inbox.view') }}</p>
                <MenuItem
                  :label="$t('inbox.viewGrouped')"
                  selection="radio"
                  :checked="settings.inboxGroupingMode === 'smart'"
                  @select="chooseGroupingMode('smart')"
                />
                <MenuItem
                  :label="$t('inbox.viewList')"
                  selection="radio"
                  :checked="settings.inboxGroupingMode === 'classic'"
                  @select="chooseGroupingMode('classic')"
                />
              </div>
            </Transition>
          </div>
        </template>
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
            :icon-size="18"
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

      <div v-if="ui.isMobile && !activeBundle" class="list-search">
        <SearchField
          :label="$t('inbox.searchPlaceholder')"
          :title="$t('inbox.searchTitle')"
          @click="ui.openOmni()"
        />
      </div>

      <!-- One strip: which mailbox the list is drawn from, then how it is
           sliced. Accounts only when there is more than one to choose. -->
      <nav
        v-if="!activeBundle && (accountFilterVisible || categoryFilterVisible)"
        class="filter-strip"
        :aria-label="$t('inbox.filterStrip')"
      >
        <template v-if="accountFilterVisible">
          <FilterPill
            :label="$t('inbox.allAccounts')"
            :pressed="!accounts.activeAccountId"
            @click="accounts.setActiveAccount('')"
          />
          <FilterPill
            v-for="account in accounts.accounts"
            :key="account.id"
            :label="account.displayName"
            :dot="account.color"
            :pressed="accounts.activeAccountId === account.id"
            @click="accounts.setActiveAccount(account.id)"
          />
        </template>
        <span v-if="accountFilterVisible && categoryFilterVisible" class="filter-strip__divider" />
        <template v-if="categoryFilterVisible">
          <FilterPill
            v-for="tab in categoryTabs"
            :key="tab.value"
            :label="tab.label"
            :count="tab.count"
            :pressed="threads.categoryFilter === tab.value"
            @click="threads.categoryFilter = tab.value"
          />
        </template>
      </nav>

      <div v-if="!activeBundle && threads.selectedThreadIds.length > 0" class="selection-tools no-drag">
        <span class="selection-count">{{ selectedCountLabel }}</span>
        <select v-model="selectedCategory" class="selection-select" :aria-label="$t('inbox.classifyAs')">
          <option
            v-for="option in classificationOptions"
            :key="option.category"
            :value="option.category"
          >
            {{ option.label }}
          </option>
        </select>
        <label class="remember-senders">
          <input v-model="rememberSelectedSenders" type="checkbox" />
          <span>{{ $t('inbox.rememberSender') }}</span>
        </label>
        <AppButton variant="primary" size="sm" :disabled="bulkBusy" @click="void classifySelected()">
          {{ $t('inbox.classify') }}
        </AppButton>
        <IconButton :title="$t('inbox.snooze')" :disabled="bulkBusy" @click="openBulkSnooze">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="8" cy="8" r="5.5"/><path d="M8 5v3.2l2.2 1.3"/></svg>
        </IconButton>
        <IconButton :title="$t('common.done')" :disabled="bulkBusy" @click="void threadActions.archive([...threads.selectedThreadIds])">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 8.4l2.5 2.5 6.5-6.8"/></svg>
        </IconButton>
      </div>

      <SnoozePicker
        v-if="bulkSnoozePickerOpen"
        :open="bulkSnoozePickerOpen"
        @choose="applyBulkSnooze"
        @close="bulkSnoozePickerOpen = false"
      />

      <!-- Thread list cell: topics folder on top, the virtualised list below -->
      <div class="min-h-0 min-w-0 overflow-hidden flex flex-col">
      <!-- Topics folder: cross-source collections, opened in the detail pane.
           Only rendered here when the list has no smart folder section to join. -->
      <div v-if="!activeBundle && !smartFolderRowVisible" class="topics-folder no-drag shrink-0">
        <TopicsFolderCard
          :topics="topicsStore.activeTopics"
          target-key="topics"
          :keyboard-focused="keyboardNavigationActive && keyboardFocusKey === 'topics'"
          @open="topicsViewOpen = true"
          @create="void createTopic()"
        />
      </div>

      <!-- Thread List -->
      <div class="min-h-0 min-w-0 flex-1 overflow-hidden">
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
          @accept-existing-senders="acceptExistingSenders"
          @bucket-action="onBucketAction"
          @group-done="onGroupDone"
          @date-group-done="onDateGroupDone"
          @new-sender-action="onNewSenderAction"
          @accept-domain="onAcceptDomain"
        />
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
        :thread-id="activeId"
        :collapse-to-latest="threads.mailboxFilter.kind === 'section' && threads.mailboxFilter.value === 'drafts'"
        :position="activeThreadPosition"
        :total="threads.scopedVisibleThreads.length"
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

.view-menu-wrap {
  position: relative;
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

.view-menu {
  position: absolute;
  top: calc(100% + 8px);
  right: 0;
  z-index: 80;
  width: 212px;
  display: grid;
  gap: 2px;
  padding: 7px;
  border: 1px solid color-mix(in srgb, var(--border-color) 84%, white 7%);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-raised) 96%, transparent);
  box-shadow: 0 22px 58px rgba(0, 0, 0, 0.46), inset 0 1px 0 rgba(255, 255, 255, 0.025);
  backdrop-filter: blur(22px) saturate(125%);
  -webkit-backdrop-filter: blur(22px) saturate(125%);
}

.view-menu-label {
  margin: 3px 10px 3px;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.menu-pop-enter-active,
.menu-pop-leave-active {
  transition: opacity var(--duration-fast) var(--ease-out), transform var(--duration-normal) var(--ease-out);
  transform-origin: top right;
}

.menu-pop-enter-from,
.menu-pop-leave-to {
  opacity: 0;
  transform: translateY(-4px) scale(0.98);
}

@media (prefers-reduced-motion: reduce) {
  .menu-pop-enter-active,
  .menu-pop-leave-active { transition: none; }
}

.list-resize {
  grid-row: 1;
}

.bundle-header {
  border-bottom: 1px solid var(--border-color-muted);
}

.inbox-layout--single {
  grid-template-columns: minmax(0, 1fr);
}

.list-search {
  padding: 0 var(--space-lg) var(--space-2-5);
  background: var(--bg-list);
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

.filter-strip__divider {
  width: 1px;
  height: 16px;
  flex-shrink: 0;
  margin: 0 var(--space-xs);
  background: var(--border-color);
}

.category-tab.active .count {
  background: color-mix(in srgb, var(--accent-primary) 18%, transparent);
  color: var(--accent-primary);
}

.selection-tools {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color-muted);
  background: color-mix(in srgb, var(--accent-primary) 7%, var(--bg-base));
  overflow-x: auto;
}

.selection-count {
  flex: 0 0 auto;
  color: var(--text-primary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-bold);
}

.selection-select {
  flex: 0 0 auto;
  min-width: 132px;
  height: 32px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-primary);
  padding: 0 8px;
  font-size: var(--font-size-xs);
}

.remember-senders {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  white-space: nowrap;
}

.remember-senders input {
  width: 14px;
  height: 14px;
  margin: 0;
  accent-color: var(--accent-primary);
}

</style>
