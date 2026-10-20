<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { useMessagesStore } from '@/stores/messages';
import { useSettingsStore } from '@/stores/settings';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useThreadsStore } from '@/stores/threads';
import { useTopicsStore } from '@/stores/topics';
import { useUiStore } from '@/stores/ui';
import { useComposerStore } from '@/stores/composer';
import { useDraftActions } from '@/composables/useDraftActions';
import { useThreadActions } from '@/composables/useThreadActions';
import { useMessageLinks } from '@/composables/useMessageLinks';
import { useAttachmentActions } from '@/composables/useAttachmentActions';
import { perfStart } from '@/utils/perf';
import { scheduleIdleWork } from '@/utils/idleWork';
import { saveTextExport } from '@/utils/saveTextExport';
import { showInFolderAction } from '@/utils/showInFolderAction';
import ErrorState from '@/components/ui/ErrorState.vue';
import ThreadMessageList from './ThreadMessageList.vue';
import ThreadPaneSkeleton from './ThreadPaneSkeleton.vue';
import ThreadToolbar from './ThreadToolbar.vue';
import ThreadPrintSheet from './ThreadPrintSheet.vue';
import ThreadMobileActions from './ThreadMobileActions.vue';
import ThreadReplyBar from './ThreadReplyBar.vue';
import TopicChip from '@/components/topics/TopicChip.vue';
import { useRouter } from 'vue-router';
import SnoozePicker from '@/components/navigation/SnoozePicker.vue';
import PluginSlotHost from '@/components/plugins/PluginSlotHost.vue';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { isOwnEmail } from '@/utils/mailIdentity';
import { threadMessageVisibility } from '@/utils/threadMessageVisibility';
import {
  buildThreadExportDescriptor,
  renderThreadMarkdown,
  selectThreadExportMessages,
} from '@/utils/threadExport';
import { registerThreadMessageKeyboardNavigation } from '@/composables/useThreadMessageKeyboardNavigation';
import { registerThreadReaderCommands } from '@/composables/useThreadReaderCommands';
import { useToast } from '@/composables/useToast';
import type { Message } from '@/types/mail';

const props = defineProps<{
  threadId?: string;
  showBack?: boolean;
  /** Keep only the latest real message and the drafts visible initially. */
  collapseToLatest?: boolean;
  /** Where this thread sits in the list that opened it, 1-based. */
  position?: number;
  total?: number;
}>();

const emit = defineEmits<{
  back: [];
  previous: [];
  next: [];
}>();

const messagesStore = useMessagesStore();
const settings = useSettingsStore();
const mailboxesStore = useMailboxesStore();
const threadsStore = useThreadsStore();
const threadActions = useThreadActions();
const messageLinks = useMessageLinks();
const draftActions = useDraftActions();
const attachmentActions = useAttachmentActions();
const accountsStore = useAccountsStore();
const connection = useConnectionStore();
const topicsStore = useTopicsStore();
const router = useRouter();
const ui = useUiStore();
const composer = useComposerStore();
const toast = useToast();
const { d, locale, t } = useI18n();
const expandedThread = ref(false);
const snoozePickerOpen = ref(false);
const printActive = ref(false);
const printSubject = ref('');
const printMessages = ref<Message[]>([]);
const PRINT_CLEANUP_FALLBACK_MS = 60_000;
let printCleanupTimer: number | null = null;
let printPreviousTitle: string | null = null;

const thread = computed(() => (props.threadId ? threadsStore.find(props.threadId) : undefined));
const threadTopics = computed(() => {
  if (!props.threadId) return [];
  return topicsStore
    .topicIdsFor('thread', props.threadId)
    .map((id) => topicsStore.byId(id))
    .filter((topic): topic is NonNullable<typeof topic> => topic !== undefined);
});
const isUnread = computed(() => (thread.value?.unreadCount ?? 0) > 0);
const messages = computed(() => (props.threadId ? messagesStore.getByThread(props.threadId) : []));
const messagesLoading = computed(() =>
  props.threadId ? messagesStore.isLoading?.(props.threadId) ?? false : false,
);
const messageVisibility = computed(() =>
  threadMessageVisibility(messages.value, {
    expanded: expandedThread.value,
    collapseToLatest: Boolean(props.collapseToLatest),
    isOwnMessage: (message) => isOwnEmail(accountsStore.accounts, message.from.email),
  }),
);
const hiddenMessageCount = computed(() => messageVisibility.value.hiddenMessageCount);
const visibleMessages = computed(() => messageVisibility.value.visibleMessages);
const draftMessages = computed(() => messageVisibility.value.draftMessages);
/** Selected, but its load waits for the selection to settle. */
const awaitingMessages = ref(false);
const initialMessagesLoading = computed(() =>
  (messagesLoading.value || awaitingMessages.value)
  && (
    messages.value.length === 0
    || [...visibleMessages.value, ...draftMessages.value]
      .some((message) => message.bodyLoaded === false)
  ),
);
/**
 * A failed message load empties the thread, which otherwise renders as a blank
 * canvas under the header — the same lie the empty thread list used to tell.
 */
const messagesError = computed(() =>
  messages.value.length === 0 ? connection.backendErrorFor('messages') : null,
);
const renderedMessageIds = computed(() => [
  ...visibleMessages.value,
  ...draftMessages.value,
].map((message) => message.id));
const messageTargets = computed(() => [
  ...messages.value.filter((message) => !message.isDraft),
  ...messages.value.filter((message) => message.isDraft),
]);
const focusedMessageId = ref('');
const messageKeyboardActive = ref(false);
/**
 * Messages opened or folded by hand. Everything else follows the default: the
 * messages shown before the thread is expanded are open, the earlier ones
 * arrive folded — moving through a long thread must not render all of it.
 */
const messageOpenOverrides = ref<Map<string, boolean>>(new Map());
const defaultOpenMessageIds = computed(() => {
  const initial = threadMessageVisibility(messages.value, {
    expanded: false,
    collapseToLatest: Boolean(props.collapseToLatest),
    isOwnMessage: (message) => isOwnEmail(accountsStore.accounts, message.from.email),
  });
  return new Set([...initial.visibleMessages, ...initial.draftMessages].map((message) => message.id));
});

function isMessageOpen(messageId: string): boolean {
  return messageOpenOverrides.value.get(messageId) ?? defaultOpenMessageIds.value.has(messageId);
}

const collapsedMessageIds = computed(() => new Set(
  messageTargets.value.filter((message) => !isMessageOpen(message.id)).map((message) => message.id),
));

function setMessagesOpen(messageIds: string[], open: boolean) {
  const next = new Map(messageOpenOverrides.value);
  for (const id of messageIds) {
    next.set(id, open);
    // An opened message waits for the observer before it mounts a renderer.
    if (open) viewportStates.delete(id);
  }
  messageOpenOverrides.value = next;
}

/*
 * Renderers mount only for open messages near the viewport (one screen above
 * and below). Without an IntersectionObserver every open message counts.
 */
const scrollRoot = ref<HTMLElement | null>(null);
const viewportStates = new Map<string, boolean>();
const viewportVersion = shallowRef(0);
let viewportObserver: IntersectionObserver | null = null;

function isNearViewport(messageId: string): boolean {
  void viewportVersion.value;
  if (typeof IntersectionObserver === 'undefined') return true;
  if (messageId === focusedMessageId.value) return true;
  return viewportStates.get(messageId) ?? defaultOpenMessageIds.value.has(messageId);
}

const bodyMountedIds = computed(() => new Set(
  renderedMessageIds.value.filter((id) => isMessageOpen(id) && isNearViewport(id)),
));

function observeMessages() {
  if (!scrollRoot.value || typeof IntersectionObserver === 'undefined') return;
  viewportObserver?.disconnect();
  viewportObserver = new IntersectionObserver((entries) => {
    let changed = false;
    for (const entry of entries) {
      const id = (entry.target as HTMLElement).dataset.threadMessageId;
      if (!id || viewportStates.get(id) === entry.isIntersecting) continue;
      viewportStates.set(id, entry.isIntersecting);
      changed = true;
    }
    if (changed) viewportVersion.value += 1;
  }, { root: scrollRoot.value, rootMargin: '100% 0px' });
  for (const element of scrollRoot.value.querySelectorAll<HTMLElement>('[data-thread-message-id]')) {
    viewportObserver.observe(element);
  }
}

watch(
  () => `${renderedMessageIds.value.join('\u0000')}\u0001${[...collapsedMessageIds.value].join('\u0000')}`,
  observeMessages,
  { flush: 'post' },
);
onMounted(observeMessages);
onBeforeUnmount(() => viewportObserver?.disconnect());

function defaultFocusedMessageId(): string {
  const targets = messageTargets.value;
  return targets.filter((message) => !message.isDraft && !message.isRead).at(-1)?.id
    ?? targets.at(-1)?.id
    ?? '';
}

function ensureFocusedMessage(): string {
  if (messageTargets.value.some((message) => message.id === focusedMessageId.value)) {
    return focusedMessageId.value;
  }
  focusedMessageId.value = defaultFocusedMessageId();
  return focusedMessageId.value;
}

async function revealFocusedMessage() {
  await nextTick();
  const target = Array.from(document.querySelectorAll<HTMLElement>('[data-thread-message-id]'))
    .find((element) => element.dataset.threadMessageId === focusedMessageId.value);
  if (!target) return;
  target.focus({ preventScroll: true });
  target.scrollIntoView({ block: 'nearest' });
}

function moveMessageFocus(delta: 1 | -1): boolean {
  const targets = messageTargets.value;
  if (targets.length === 0) return false;

  // Reveals the earlier messages, folded; opening one is Enter or `o`.
  expandedThread.value = true;
  const currentId = ensureFocusedMessage();
  const currentIndex = Math.max(0, targets.findIndex((message) => message.id === currentId));
  const nextIndex = Math.min(targets.length - 1, Math.max(0, currentIndex + delta));
  focusedMessageId.value = targets[nextIndex]?.id ?? focusedMessageId.value;
  messageKeyboardActive.value = true;
  void revealFocusedMessage();
  return true;
}

function toggleFocusedMessage(activateKeyboard = true): boolean {
  const messageId = ensureFocusedMessage();
  if (!messageId) return false;

  expandedThread.value = true;
  setMessagesOpen([messageId], !isMessageOpen(messageId));
  messageKeyboardActive.value = activateKeyboard;
  if (activateKeyboard) void revealFocusedMessage();
  return true;
}

function toggleAllMessages(): boolean {
  const ids = messageTargets.value.map((message) => message.id);
  if (ids.length === 0) return false;

  expandedThread.value = true;
  setMessagesOpen(ids, ids.every((id) => !isMessageOpen(id)));
  ensureFocusedMessage();
  messageKeyboardActive.value = true;
  void revealFocusedMessage();
  return true;
}

function openFocusedMessage(): boolean {
  const messageId = ensureFocusedMessage();
  if (!messageId) return false;
  const message = messageTargets.value.find((candidate) => candidate.id === messageId);
  if (message?.isDraft) composer.openDraft(message.id);
  else toggleFocusedMessage();
  return true;
}

function focusMessageFromPointer(messageId: string) {
  focusedMessageId.value = messageId;
  dismissMessageKeyboardFocus();
}

function toggleMessageFromPointer(messageId: string) {
  focusedMessageId.value = messageId;
  toggleFocusedMessage(false);
}

function dismissMessageKeyboardFocus() {
  if (!messageKeyboardActive.value) return;
  messageKeyboardActive.value = false;
  const focused = document.activeElement as HTMLElement | null;
  if (focused?.matches('[data-thread-message-id]')) focused.blur();
}

onMounted(() => {
  window.addEventListener('pointerdown', dismissMessageKeyboardFocus, true);
  window.addEventListener('pointermove', dismissMessageKeyboardFocus, { passive: true });
});

onBeforeUnmount(() => {
  window.removeEventListener('pointerdown', dismissMessageKeyboardFocus, true);
  window.removeEventListener('pointermove', dismissMessageKeyboardFocus);
});

onBeforeUnmount(registerThreadMessageKeyboardNavigation({
  move: moveMessageFocus,
  open: openFocusedMessage,
  toggleFocused: toggleFocusedMessage,
  toggleAll: toggleAllMessages,
}));

watch(messageTargets, () => { ensureFocusedMessage(); }, { immediate: true });

// Open messages near the viewport need their body and inline images. A
// refresh keeps loaded bodies, so this only fires for what is actually new.
watch(
  [
    () => props.threadId,
    messagesLoading,
    awaitingMessages,
    () => [...visibleMessages.value, ...draftMessages.value]
      .filter((message) => bodyMountedIds.value.has(message.id))
      .filter((message) => (
        (message.bodyLoaded === false && !messagesStore.isBodyLoading(message.id))
        || (message.bodyLoaded !== false && messagesStore.needsDetails(message.id))
      ))
      .map((message) => message.id)
      .join('\u0000'),
  ],
  ([threadId, loading, awaiting, key]) => {
    if (!threadId || loading || awaiting || !key) return;
    void messagesStore.loadAttachmentsForMessages(threadId, key.split('\u0000'));
  },
  { flush: 'post' },
);
const threadAccount = computed(() =>
  accountsStore.accounts.find((account) => account.id === thread.value?.accountId),
);
const canReply = computed(() => accountsStore.capabilitiesFor(threadAccount.value).can_reply);
const latestSenderAddress = computed(() => visibleMessages.value.at(-1)?.from.email ?? '');

/** Who the reply would go to: the last sender who is not one of our own. */
const replyRecipientName = computed(() => {
  const incoming = [...visibleMessages.value]
    .reverse()
    .find((message) => !isOwnEmail(accountsStore.accounts, message.from.email));
  const from = incoming?.from ?? visibleMessages.value.at(-1)?.from;
  return from?.name?.trim() || from?.email || '';
});
const moveTargets = computed(() => {
  if (!thread.value) return [];
  const currentNames = new Set(thread.value.mailboxNames.map((name) => name.trim().toLowerCase()));
  return mailboxesStore.mailboxes
    .filter((mailbox) => mailbox.account_id === thread.value?.accountId)
    .filter((mailbox) => !currentNames.has(mailbox.name.trim().toLowerCase()))
    .map((mailbox) => ({ id: mailbox.id, label: mailbox.name }))
    .sort((left, right) => left.label.localeCompare(right.label, locale.value, { sensitivity: 'base' }));
});
let markReadTimer: ReturnType<typeof setTimeout> | null = null;
let settleTimer: ReturnType<typeof setTimeout> | null = null;
let cancelPrefetch: (() => void) | null = null;
let releaseThread: (() => void) | null = null;
/** Selection changes closer together than this are scanning, not reading. */
const SELECTION_SETTLE_MS = 100;
const PREFETCH_AFTER_REST_MS = 400;

function clearThreadTimers() {
  if (markReadTimer) clearTimeout(markReadTimer);
  if (settleTimer) clearTimeout(settleTimer);
  cancelPrefetch?.();
  markReadTimer = null;
  settleTimer = null;
  cancelPrefetch = null;
}

onBeforeUnmount(() => {
  clearThreadTimers();
  releaseThread?.();
  cleanupPrintView();
});

/** The neighbour one further in the direction the selection last moved. */
function neighbourInTravel(previousId: string | undefined, nextId: string): string | undefined {
  const order = threadsStore.scopedVisibleThreads.map((item) => item.id);
  const index = order.indexOf(nextId);
  if (index < 0) return undefined;
  const previousIndex = previousId ? order.indexOf(previousId) : -1;
  const direction = previousIndex > index ? -1 : 1;
  return order[index + direction];
}

async function openSelectedThread(threadId: string, previousId: string | undefined) {
  settleTimer = null;
  const endOpen = perfStart(`thread.open:${threadId}`);
  awaitingMessages.value = !messagesStore.hasThread(threadId);
  if (!topicsStore.hasMembershipsFor('thread', threadId)) {
    void topicsStore.loadMembershipsFor('thread', [threadId]);
  }
  await messagesStore.openThread(threadId);
  if (props.threadId !== threadId) return;
  awaitingMessages.value = false;
  endOpen?.();
  const current = threadsStore.find(threadId);
  if (current?.unreadCount) {
    // Dwell before marking read: this watcher also fires for auto-selection
    // (app launch, archive advancing to the next thread, j/k scanning) — an
    // instant mark-read would systematically destroy unread state for
    // threads the user never actually looked at.
    markReadTimer = setTimeout(() => {
      markReadTimer = null;
      if (props.threadId === threadId) {
        void threadActions.setRead([threadId], true);
      }
    }, 800);
  }
  const neighbour = neighbourInTravel(previousId, threadId);
  if (neighbour) {
    // Only once the reader rests: a prefetched neighbour opens without the
    // settle delay, so prefetching while `j` is held would chain one load per key.
    const restTimer = setTimeout(() => {
      cancelPrefetch = scheduleIdleWork(() => {
        cancelPrefetch = null;
        if (props.threadId === threadId) void messagesStore.prefetchThread(neighbour);
      });
    }, PREFETCH_AFTER_REST_MS);
    cancelPrefetch = () => clearTimeout(restTimer);
  }
}

watch(
  () => props.threadId,
  (next, previous) => {
    clearThreadTimers();
    releaseThread?.();
    releaseThread = null;
    expandedThread.value = false;
    focusedMessageId.value = '';
    messageKeyboardActive.value = false;
    messageOpenOverrides.value = new Map();
    viewportStates.clear();
    awaitingMessages.value = false;
    if (!next) {
      return;
    }
    releaseThread = messagesStore.retainThread(next);
    // The list highlight moves at once; the load waits until the selection
    // rests, so holding `j` does not fetch every thread it passes.
    if (!previous || messagesStore.hasThread(next)) {
      void openSelectedThread(next, previous);
      return;
    }
    awaitingMessages.value = true;
    settleTimer = setTimeout(() => void openSelectedThread(next, previous), SELECTION_SETTLE_MS);
  },
  { immediate: true },
);

/** Same path the thread opens on, so a retry cannot drift from the first load. */
async function reloadMessages() {
  if (!props.threadId) return;
  await messagesStore.loadByThread(props.threadId);
}

function archiveThread() {
  const current = thread.value;
  if (!current || current.workflowState === 'done') return;

  // The shared action applies its optimistic list state before its first await. A
  // routed phone detail can therefore close in the same tap while archive and
  // persistence continue in the background (and roll back with a toast on
  // failure).
  void threadActions.archive([current.id]);
  if (props.showBack) {
    emit('back');
  }
}

/**
 * Discard a draft that sits inside this conversation. The thread keeps its
 * remaining messages, so both the message list and the thread projection
 * (count, snippet, date) have to be re-read afterwards.
 */
async function discardDraft(messageId: string) {
  const threadId = props.threadId;
  if (!threadId) return;
  if (!(await draftActions.discard(messageId))) return;
  await Promise.all([messagesStore.loadByThread(threadId), threadsStore.load()]);
}

async function deleteThread() {
  if (!thread.value) return;
  threadsStore.focusNeighborThread(thread.value.id);
  // Shared with the list and the keyboard path: undo toast on success, error
  // toast on failure. Calling the store directly used to reject unhandled,
  // which rolled the thread back into the list without a word.
  await threadActions.trash([thread.value.id]);
}

function snoozeThread() {
  snoozePickerOpen.value = true;
}

function openTopicPicker() {
  if (!thread.value) return;
  topicsStore.openPicker({ itemType: 'thread', itemIds: [thread.value.id] });
}

function openTopic(topicId: string) {
  void router.push({ name: 'topic', params: { id: topicId } });
}

function removeFromTopic(topicId: string) {
  if (!thread.value) return;
  void threadActions.removeFromTopic([thread.value.id], topicId);
}

function applySnooze(date: string) {
  if (!thread.value) return;
  snoozePickerOpen.value = false;
  void threadActions.snoozeAt([thread.value.id], date);
}

async function organizeThread(kind: 'tags' | 'assignee' | 'sharing') {
  if (!thread.value) return;
  try {
    if (kind === 'tags') {
      const next = window.prompt(t('thread.organize.tagsPrompt'), thread.value.tags.join(', '));
      if (next === null) return;
      await threadsStore.setTagsForThread(thread.value.id, next.split(','));
    } else if (kind === 'assignee') {
      const next = window.prompt(t('thread.organize.assigneePrompt'), thread.value.assignee ?? '');
      if (next === null) return;
      await threadsStore.setAssignee(thread.value.id, next);
    } else {
      const next = window.prompt(
        t('thread.organize.sharingPrompt'),
        (thread.value.sharedWith ?? []).join(', '),
      );
      if (next === null) return;
      await threadsStore.setSharedWith(thread.value.id, next.split(','));
    }
    toast.success(t('thread.organize.saved'));
  } catch (error) {
    toast.error(t('thread.organize.saveFailed', { error: String(error) }));
  }
}

async function toggleReadState() {
  if (!thread.value) return;
  await threadActions.setRead([thread.value.id], isUnread.value);
}

function replyThread() {
  if (!canReply.value || !thread.value) return;
  composer.openReply(thread.value.id);
}

function forwardThread() {
  if (!canReply.value || !thread.value) return;
  composer.openForward(thread.value.id);
}

function resendThread() {
  if (!canReply.value || !thread.value) return;
  composer.openResend(thread.value.id);
}

async function toggleMuted() {
  if (!thread.value) return;
  await threadActions.setMuted(thread.value.id, !thread.value.isMuted);
}

async function markSpam() {
  if (!thread.value) return;
  await threadActions.markSpam([thread.value.id]);
}

async function togglePin() {
  if (!thread.value) return;
  await threadActions.togglePinned(thread.value.id);
}

async function toggleAside() {
  if (!thread.value) return;
  if (thread.value.workflowState === 'aside') {
    await threadActions.moveToInbox([thread.value.id]);
  } else {
    await threadActions.setAside([thread.value.id]);
  }
}

async function blockSender(scope: 'address' | 'domain' = 'address') {
  if (!thread.value) return;
  const lastMsg = messagesStore.getByThread(thread.value.id).at(-1);
  const sender = lastMsg?.from?.email;
  if (!sender) return;
  const domain = sender.split('@').at(-1)?.trim();
  const pattern = scope === 'domain' && domain ? `*@${domain}` : sender;
  const confirmationKey = scope === 'domain' && domain
    ? 'thread.block.confirmDomain'
    : 'thread.block.confirmAddress';
  if (!window.confirm(t(confirmationKey, { address: sender, domain })))
    return;
  try {
    await threadActions.blockSender({
      pattern,
      accountId: thread.value.accountId,
      threadIds: [thread.value.id],
    });
    toast.success(scope === 'domain'
      ? t('thread.block.domainBlocked', { domain })
      : t('thread.block.senderBlocked', { address: sender }));
  } catch (err) {
    toast.error(t('thread.block.failed', { error: err instanceof Error ? err.message : String(err) }));
  }
}

async function moveThread(targetFolderId: string) {
  if (!thread.value) return;
  const target = moveTargets.value.find((item) => item.id === targetFolderId);
  const currentId = thread.value.id;
  threadsStore.focusNeighborThread(currentId);
  await threadActions.moveToFolder(
    [currentId],
    targetFolderId,
    target?.label,
  );
}

onBeforeUnmount(registerThreadReaderCommands({
  get threadId() { return thread.value?.id ?? ''; },
  get moveTargets() { return moveTargets.value; },
  get isMuted() { return Boolean(thread.value?.isMuted); },
  move: moveThread,
  toggleMuted,
  markSpam,
  blockSender,
  print: printThread,
  exportThread,
}));

function threadMessages(): readonly Message[] {
  return thread.value ? messagesStore.getByThread(thread.value.id) : [];
}

function selectedMessages(scope: 'latest' | 'conversation'): readonly Message[] {
  return selectThreadExportMessages(threadMessages(), scope);
}

function cleanupPrintView() {
  if (typeof window !== 'undefined') {
    window.removeEventListener('afterprint', cleanupPrintView);
    if (printCleanupTimer !== null) {
      window.clearTimeout(printCleanupTimer);
      printCleanupTimer = null;
    }
  }
  if (typeof document !== 'undefined') {
    document.body.classList.remove('satchel-thread-printing');
    if (printPreviousTitle !== null) {
      document.title = printPreviousTitle;
      printPreviousTitle = null;
    }
  }
  printActive.value = false;
  printSubject.value = '';
  printMessages.value = [];
}

function formatPrintDate(value: string) {
  try {
    return new Date(value).toLocaleString(locale.value);
  } catch {
    return value;
  }
}

function printRecipientLine(message: Message) {
  return (message.to ?? []).map((participant) => participant.email).join(', ');
}

async function printThread(scope: 'latest' | 'conversation') {
  if (typeof window === 'undefined') return;
  const currentThread = thread.value;
  if (!currentThread) return;
  cleanupPrintView();
  printSubject.value = currentThread.subject;
  printMessages.value = [...selectedMessages(scope)];
  printActive.value = true;
  printPreviousTitle = document.title;
  document.title = currentThread.subject;
  document.body.classList.add('satchel-thread-printing');

  try {
    await nextTick();
    window.addEventListener('afterprint', cleanupPrintView, { once: true });
    printCleanupTimer = window.setTimeout(cleanupPrintView, PRINT_CLEANUP_FALLBACK_MS);
    window.print();
  } catch {
    cleanupPrintView();
    toast.error(t('thread.print.failed'));
  }
}

function messagesAsMarkdown(scope: 'latest' | 'conversation') {
  const currentThread = thread.value;
  if (!currentThread) return '';
  return renderThreadMarkdown(currentThread.subject, threadMessages(), scope, threadExportOptions());
}

function threadExportOptions() {
  return {
    labels: {
      from: t('common.from'),
      to: t('common.to'),
      cc: t('common.cc'),
      date: t('common.date'),
    },
    formatDate: (date: Date) => d(date, 'dateTime'),
    filenameFallback: t('thread.export.filenameFallback'),
  };
}

async function copyMarkdown(scope: 'latest' | 'conversation') {
  const content = messagesAsMarkdown(scope);
  if (!content) return;
  try {
    await navigator.clipboard.writeText(content);
    toast.success(scope === 'latest'
      ? t('thread.export.copiedLatest')
      : t('thread.export.copiedConversation'));
  } catch (error) {
    toast.error(t('thread.export.copyFailed', { error: error instanceof Error ? error.message : String(error) }));
  }
}

const downloadableAttachments = computed(() =>
  attachmentActions.listDownloadableAttachments(messages.value),
);

async function saveAllAttachments() {
  const attachments = downloadableAttachments.value;
  if (attachments.length === 0) return;

  try {
    const result = await attachmentActions.saveAttachmentsToDirectory(attachments, {
      fallbackFilename: t('thread.attachments.filenameFallback'),
    });
    if (!result) return;
    const { directory, saved, failed } = result;
    const failedNames = failed.map(
      (attachment) => attachment.filename || t('thread.attachments.unnamed'),
    );

    if (saved > 0 && failed.length === 0) {
      toast.success(t('thread.attachments.saved', { count: saved }), {
        action: showInFolderAction(directory, t('thread.attachments.openFolder')),
      });
    } else if (saved > 0) {
      toast.info(t('thread.attachments.savedPartial', {
        saved,
        failed: failed.length,
        names: failedNames.join(', '),
      }), {
        action: showInFolderAction(directory, t('thread.attachments.openFolder')),
      });
    } else {
      toast.error(t('thread.attachments.saveFailedAll'));
    }
  } catch (error) {
    toast.error(t('thread.attachments.saveFailed', { error: String(error) }));
  }
}

async function exportThread(format: 'eml' | 'markdown' | 'text') {
  const currentThread = thread.value;
  if (!currentThread) return;
  const options = buildThreadExportDescriptor(
    format,
    currentThread.subject,
    threadMessages(),
    threadExportOptions(),
  );
  try {
    const saved = await saveTextExport(options.filename, options.content, options.mime, options.extension);
    if (saved) {
      toast.success(t('thread.export.exported', { filename: options.filename }), {
        action: typeof saved === 'string' ? showInFolderAction(saved) : undefined,
      });
    }
  } catch (error) {
    toast.error(t('thread.export.failed', { error: error instanceof Error ? error.message : String(error) }));
  }
}

function showEntireThread() {
  expandedThread.value = true;
}
</script>

<template>
  <section v-if="thread" class="flex flex-col h-full overflow-hidden bg-surface">
    <!-- ═══ DESKTOP TOOLBAR ═══ -->
    <ThreadToolbar
      v-if="!ui.isMobile"
      :thread-id="thread.id"
      :subject="thread.subject"
      :is-unread="isUnread"
      :is-starred="thread.isStarred"
      :is-aside="thread.workflowState === 'aside'"
      :is-muted="thread.isMuted"
      :show-back="showBack"
      :position="props.position ?? 0"
      :total="props.total ?? 0"
      :sender-address="latestSenderAddress"
      :move-targets="moveTargets"
      :can-reply="canReply"
      @back="emit('back')"
      @previous="emit('previous')"
      @next="emit('next')"
      @archive="archiveThread"
      @toggle-read="void toggleReadState()"
      @toggle-pin="togglePin"
      @set-aside="toggleAside"
      @snooze="snoozeThread"
      @delete="deleteThread"
      @toggle-muted="toggleMuted"
      @spam="markSpam"
      @block="blockSender"
      @move="moveThread"
      @print="printThread"
      :attachment-count="downloadableAttachments.length"
      :reader-zoom="settings.readerZoom"
      @zoom="(delta) => settings.setReaderZoom(delta === 0 ? 100 : settings.readerZoom + delta)"
      @save-attachments="void saveAllAttachments()"
      @copy-markdown="copyMarkdown"
      @copy-link="void messageLinks.copy(thread.id)"
      @export="exportThread"
      @reply="replyThread"
      @resend="resendThread"
      @forward="forwardThread"
      @all-actions="ui.openOmni()"
      @organize="void organizeThread($event)"
      @add-to-topic="openTopicPicker"
    />

    <!-- ═══ MOBILE TOP BAR ═══ -->
    <div v-else class="mobile-top-bar" data-tauri-drag-region>
      <IconButton class="mobile-back-btn" :title="$t('common.back')" :aria-label="$t('common.back')" @click="emit('back')">
        <svg width="22" height="22" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10 3L5 8l5 5"/>
        </svg>
      </IconButton>
      <div class="mobile-top-meta">
        <span class="mobile-top-count">{{ $t('thread.row.messages', { count: messages.length }) }}</span>
      </div>
    </div>

    <!-- ═══ SCROLLABLE CONTENT ═══ -->
    <div ref="scrollRoot" :class="['flex-1 overflow-y-auto', ui.isMobile ? 'px-4 py-5' : 'thread-reading-content']">
      <div :class="ui.isMobile ? '' : 'max-w-3xl mx-auto'">
        <PluginSlotHost
          slot-name="thread_view_panel"
          :context="{ threadId: thread.id, subject: thread.subject }"
        />
        <!-- Thread heading -->
        <div :class="ui.isMobile ? 'mb-5' : 'mb-6'">
          <div class="thread-heading-line">
            <h1 class="text-2xl font-semibold leading-tight tracking-tight text-primary">{{ thread.normalizedSubject ?? thread.subject }}</h1>
            <span v-if="thread.workflowState === 'done'" class="thread-state-badge">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 8.5l3.5 3.5L13 5"/>
              </svg>
              {{ $t('common.done') }}
            </span>
          </div>
          <!-- Which mailbox this arrived in, and how long the thread is. Both
               are answered by scrolling; the line saves the scroll. -->
          <div class="mt-1.5 flex flex-wrap items-center gap-2.5 text-xs text-tertiary">
            <span v-if="threadAccount" class="inline-flex items-center gap-1.5">
              <span
                class="h-[7px] w-[7px] rounded-full"
                :style="{ background: threadAccount.color }"
                aria-hidden="true"
              />
              {{ threadAccount.displayName }}
            </span>
            <span>{{ $t('thread.row.messages', { count: messages.length }) }}</span>
          </div>
          <div v-if="threadTopics.length > 0" class="mt-2 flex flex-wrap items-center gap-1.5">
            <TopicChip
              v-for="topic in threadTopics"
              :key="topic.id"
              :label="topic.name"
              :color="topic.color"
              removable
              @click="openTopic(topic.id)"
              @remove="removeFromTopic(topic.id)"
            />
          </div>
        </div>

        <!-- Messages -->
        <div v-if="hiddenMessageCount > 0" class="mb-5">
          <AppButton

            @click="showEntireThread"
            variant="secondary"
            size="sm"
          >
            {{ $t('thread.view.showEarlier', { count: hiddenMessageCount }) }}
          </AppButton>
        </div>

        <ThreadPaneSkeleton v-if="initialMessagesLoading" compact />
        <ErrorState
          v-else-if="messagesError"
          :title="$t('thread.view.loadFailed')"
          :detail="backendErrorLabel(messagesError)"
          @retry="void reloadMessages()"
        />
        <ThreadMessageList
          v-else
          :messages="visibleMessages"
          :draft-messages="draftMessages"
          :can-reply="canReply"
          :draft-busy="draftActions.isBusy"
          :collapsed-message-ids="collapsedMessageIds"
          :body-mounted-ids="bodyMountedIds"
          :focused-message-id="focusedMessageId"
          :keyboard-active="messageKeyboardActive"
          @reply="replyThread"
          @forward="forwardThread"
          @edit-draft="composer.openDraft($event)"
          @discard-draft="void discardDraft($event)"
          @focus-message="focusMessageFromPointer"
          @toggle-collapsed="toggleMessageFromPointer"
        />
      </div>
    </div>

    <!-- The pane ends where an answer would begin. -->
    <div v-if="canReply && !ui.isMobile" class="thread-reply-foot">
      <ThreadReplyBar
        :recipient="replyRecipientName"
        :identity="threadAccount?.displayName ?? ''"
        :identity-color="threadAccount?.color ?? null"
        @reply="replyThread"
      />
    </div>

    <ThreadMobileActions
      :visible="ui.isMobile"
      :can-reply="canReply"
      :is-unread="isUnread"
      :is-starred="Boolean(thread.isStarred)"
      :is-aside="thread.workflowState === 'aside'"
      :is-done="thread.workflowState === 'done'"
      @archive="archiveThread"
      @reply="replyThread"
      @toggle-read="void toggleReadState()"
      @snooze="snoozeThread"
      @toggle-pin="void togglePin()"
      @set-aside="void toggleAside()"
      @forward="forwardThread"
      @delete="void deleteThread()"
    />

    <SnoozePicker
      :open="snoozePickerOpen"
      @choose="applySnooze"
      @close="snoozePickerOpen = false"
    />
  </section>

  <!-- Empty State -->
  <section v-else-if="!thread" class="h-full flex flex-col items-center justify-center gap-3 bg-surface text-tertiary text-sm">
    <svg width="40" height="40" viewBox="0 0 40 40" fill="none" stroke="currentColor" stroke-width="1" opacity="0.15">
      <rect x="5" y="10" width="30" height="22" rx="4"/>
      <path d="M5 16L20 24L35 16"/>
    </svg>
    <p>{{ $t('thread.view.empty') }}</p>
  </section>

  <ThreadPrintSheet
    :active="printActive"
    :subject="printSubject"
    :messages="printMessages"
    :format-date="formatPrintDate"
    :recipient-line="printRecipientLine"
  />
</template>

<style scoped>
.thread-reading-content {
  padding: var(--space-xl) calc(var(--space-5) * 2) var(--space-2xl);
}

.thread-reply-foot {
  flex-shrink: 0;
  box-sizing: border-box;
  width: 100%;
  max-width: 48rem;
  margin: 0 auto;
  padding: 0 calc(var(--space-5) * 2) var(--space-5);
}

.thread-heading-line {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-sm);
}

.thread-state-badge {
  display: inline-flex;
  align-items: center;
  gap: var(--space-xs);
  min-height: 24px;
  padding: 2px 8px;
  border: 1px solid color-mix(in srgb, var(--accent-green) 35%, transparent);
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-green) 10%, transparent);
  color: var(--accent-green);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
}

/* ── Mobile top bar ── */
.mobile-top-bar {
  height: 52px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding: 0 var(--space-sm);
  background: var(--bg-base);
  border-bottom: 1px solid var(--border-color-muted);
}

.mobile-back-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  background: none;
  border: none;
  cursor: pointer;
  -webkit-tap-highlight-color: transparent;
  transition: background var(--duration-fast) var(--ease-out);
}

.mobile-back-btn:active {
  background: var(--bg-hover);
}

.mobile-top-meta {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.mobile-top-count {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  font-weight: var(--weight-medium);
}

/* ── Desktop reply dock ── */

/* Quote bar */

/* Fields */

/* ── Mobile reply sheet ── */

/* ── Drag regions ── */
[data-tauri-drag-region] {
  -webkit-app-region: drag;
}

[data-tauri-drag-region] button {
  -webkit-app-region: no-drag;
}
</style>
