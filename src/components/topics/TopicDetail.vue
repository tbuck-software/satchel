<script setup lang="ts">
import { useConnectionStore } from '@/stores/connection';
import { useAccountsStore } from '@/stores/accounts';
import CardButton from '@/components/ui/CardButton.vue';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { saveAttachmentWithDialog } from '@/utils/saveAttachment';
import { useThreadReply } from '@/composables/useThreadReply';
import { useMessagesStore } from '@/stores/messages';
import { useTopicsStore, type TopicFile, type TopicTimelineItem } from '@/stores/topics';
import { useToast } from '@/composables/useToast';
import type { Attachment, Message } from '@/types/mail';
import { isListWorthyAttachment } from '@/utils/attachments';
import { formatBytes } from '@/utils/formatBytes';
import { createHydrationQueue } from '@/utils/hydrationQueue';
import AttachmentChip from '@/components/mail/AttachmentChip.vue';
import Avatar from '@/components/common/Avatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import IconButton from '@/components/ui/IconButton.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import SkeletonRow from '@/components/ui/SkeletonRow.vue';
import SnoozePicker from '@/components/navigation/SnoozePicker.vue';
import TopicFeedItem from '@/components/topics/TopicFeedItem.vue';

const accounts = useAccountsStore();
const connection = useConnectionStore();
watch(() => accounts.visibilityKey, () => { timeline.value = []; void loadTimeline(); });
let timelineRequest = 0;

const props = defineProps<{
  topicId: string;
}>();

const emit = defineEmits<{
  /** The topic was deleted; the host decides where to go. */
  deleted: [];
  /** A linked conversation should be opened; the host decides how. */
  openThread: [threadId: string];
}>();

const topics = useTopicsStore();
const messagesStore = useMessagesStore();
const threadReply = useThreadReply();
const toast = useToast();
const { locale, t } = useI18n();

const topicId = computed(() => props.topicId);
const topic = computed(() => topics.byId(topicId.value));
const isDone = computed(() => topic.value?.status !== 'active');

const timeline = ref<TopicTimelineItem[]>([]);
const timelineLoading = ref(false);
const filter = ref<'all' | 'mail' | 'notes'>('all');
const noteDraft = ref('');
const descriptionDraft = ref('');
const editingTitle = ref(false);
const titleDraft = ref('');
const titleInput = ref<HTMLInputElement | null>(null);
const editingIcon = ref(false);
const iconDraft = ref('');
const iconInput = ref<HTMLInputElement | null>(null);
const confirmingDelete = ref(false);

const filterOptions = computed(() => [
  { value: 'all', label: t('common.all') },
  { value: 'mail', label: t('topics.detail.filterMail') },
  { value: 'notes', label: t('topics.detail.filterNotes') },
]);

interface FeedEntry {
  key: string;
  item: TopicTimelineItem;
  /** Present for mail entries — one card per message, like issue comments. */
  message?: Message;
  date: string;
}

/**
 * The issue feed, oldest → newest. Linked conversations are flattened into
 * one entry per message so every mail reads like a comment; nothing nests.
 */
const feed = computed<FeedEntry[]>(() => {
  const entries: FeedEntry[] = [];
  const seenMessageIds = new Set<string>();
  for (const item of timeline.value) {
    if (item.item_type === 'note') {
      if (filter.value === 'mail') continue;
      entries.push({ key: `note:${item.item_id}`, item, date: item.sort_date });
      continue;
    }
    if (filter.value === 'notes') continue;
    const messages = messagesFor(item);
    if (messages.length === 0) {
      // Still loading: keep the slot so the feed doesn't jump around.
      entries.push({ key: `pending:${item.item_id}`, item, date: item.sort_date });
      continue;
    }
    for (const message of messages) {
      // A thread link and a single-message link can overlap; one card per mail.
      if (seenMessageIds.has(message.id)) continue;
      seenMessageIds.add(message.id);
      if (participantFilter.value && message.from.email.toLowerCase() !== participantFilter.value) {
        continue;
      }
      entries.push({ key: `mail:${message.id}`, item, message, date: message.timestamp });
    }
  }
  return entries.sort((a, b) => a.date.localeCompare(b.date));
});

/** Everyone who wrote a mail in this topic; clicking an avatar filters the feed. */
const participants = computed(() => {
  const seen = new Map<string, { name: string; email: string }>();
  for (const item of timeline.value) {
    if (item.item_type === 'note' || !item.thread_id) continue;
    for (const message of messagesFor(item)) {
      const email = message.from.email.toLowerCase();
      if (!email || seen.has(email)) continue;
      seen.set(email, { name: message.from.name || message.from.email, email: message.from.email });
    }
  }
  return [...seen.values()];
});

const participantFilter = ref('');

function toggleParticipant(email: string) {
  participantFilter.value = participantFilter.value === email.toLowerCase() ? '' : email.toLowerCase();
}

/** Every attachment across the linked mails (unaffected by feed filters). */
const allAttachments = computed<Attachment[]>(() => {
  const seen = new Set<string>();
  const collected: Attachment[] = [];
  for (const item of timeline.value) {
    for (const message of messagesFor(item)) {
      for (const attachment of message.attachments) {
        if (seen.has(attachment.id) || !isListWorthyAttachment(attachment)) continue;
        seen.add(attachment.id);
        collected.push(attachment);
      }
    }
  }
  return collected;
});

function messagesFor(item: TopicTimelineItem): Message[] {
  if (item.item_type === 'note' || !item.thread_id) return [];
  const threadMessages = messagesStore.getByThread(item.thread_id);
  if (item.item_type === 'message') {
    return threadMessages.filter((message) => message.id === item.item_id);
  }
  return threadMessages;
}

// ── Scroll: open at the newest entry, keep a jump-back bar to the head ─────
const scrollEl = ref<HTMLElement | null>(null);
const scrolledAway = ref(false);

function onScroll() {
  scrolledAway.value = (scrollEl.value?.scrollTop ?? 0) > 180;
}

function scrollToTop() {
  scrollEl.value?.scrollTo({ top: 0, behavior: 'smooth' });
}

function scrollToBottom() {
  const el = scrollEl.value;
  if (el) el.scrollTo({ top: el.scrollHeight });
}

const topicFiles = computed(() => topics.filesFor(topicId.value));
const fileInput = ref<HTMLInputElement | null>(null);
const uploadingFiles = ref(false);

function loadTopicFiles() {
  return topics.loadFiles(topicId.value);
}

function pickFiles() {
  fileInput.value?.click();
}

/** Read the picked files and store them on the topic. */
async function onFilesPicked(event: Event) {
  const input = event.target as HTMLInputElement;
  const files = [...(input.files ?? [])];
  input.value = '';
  if (files.length === 0 || !topicId.value) return;
  uploadingFiles.value = true;
  try {
    for (const file of files) {
      const buffer = new Uint8Array(await file.arrayBuffer());
      let binary = '';
      const chunk = 0x8000;
      for (let offset = 0; offset < buffer.length; offset += chunk) {
        binary += String.fromCharCode(...buffer.subarray(offset, offset + chunk));
      }
      await topics.addFile({
        topicId: topicId.value,
        filename: file.name,
        mimeType: file.type || undefined,
        contentBase64: btoa(binary),
      });
    }
    toast.success(
      files.length === 1
        ? t('topics.toast.fileAdded')
        : t('topics.toast.filesAdded', { count: files.length }),
    );
  } catch (error) {
    toast.error(t('topics.toast.fileSaveFailed', { error: String(error) }));
  } finally {
    uploadingFiles.value = false;
  }
}

async function downloadTopicFile(file: TopicFile) {
  try {
    const data = await topics.downloadFile(file.id);
    await saveAttachmentWithDialog(data.filename || file.filename, data.data_base64);
  } catch (error) {
    toast.error(t('topics.toast.fileSaveFailed', { error: String(error) }));
  }
}

async function deleteTopicFile(file: TopicFile) {
  try {
    await topics.deleteFile(topicId.value, file.id);
  } catch {
    toast.error(t('topics.toast.fileDeleteFailed'));
  }
}

async function loadTimeline(jumpToEnd = false) {
  if (!topicId.value) return;
  const request = ++timelineRequest;
  const scope = accounts.visibilityKey;
  const id = topicId.value;
  timelineLoading.value = true;
  try {
    const rows = await topics.fetchTimeline(id);
    if (request !== timelineRequest || scope !== accounts.visibilityKey || id !== topicId.value) return;
    timeline.value = rows;
    connection.clearBackendError('topics');
  } catch (error) {
    if (request === timelineRequest) connection.reportBackendError('topics', error);
    return;
  } finally {
    if (request === timelineRequest) timelineLoading.value = false;
  }
  void loadTopicFiles();
  if (jumpToEnd) {
    // Once after render, once after the mail bodies hydrated and grew.
    await nextTick();
    scrollToBottom();
    window.setTimeout(scrollToBottom, 450);
  }
  // The cards show the snippet until their conversation arrives. Which ones
  // load, and when, is the observer's business below.
  hydration.reset();
  await nextTick();
  observeCards();
}

/*
 * Embedded conversations load when their card nears the viewport, three at a
 * time. Starting every linked thread at once cost twenty loads — and, before
 * the batch listing, some 300 IPC rounds — for a message cache that keeps
 * three threads and threw most of it away again.
 *
 * loadByThread hydrates the full body of the latest mail only, so the rest
 * are hydrated explicitly: the feed shows every mail's content.
 */
const hydration = createHydrationQueue((threadId) =>
  messagesStore.loadByThread(threadId).then(() => {
    const ids = messagesStore.getByThread(threadId).map((message) => message.id);
    return messagesStore.loadAttachmentsForMessages(threadId, ids);
  }),
);
/** How far outside the viewport a card starts loading, so scrolling finds it ready. */
const CARD_OVERSCAN = '600px 0px';
let cardObserver: IntersectionObserver | null = null;

function observeCards() {
  cardObserver?.disconnect();
  cardObserver = null;
  const root = scrollEl.value;
  if (!root) return;
  const cards = [...root.querySelectorAll<HTMLElement>('[data-thread-id]')];
  if (typeof IntersectionObserver === 'undefined') {
    // No observer to ask: every card, still through the limiter.
    for (const card of cards) hydration.request(card.dataset.threadId ?? '');
    return;
  }
  cardObserver = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          hydration.request((entry.target as HTMLElement).dataset.threadId ?? '');
        }
      }
    },
    { root, rootMargin: CARD_OVERSCAN },
  );
  for (const card of cards) cardObserver.observe(card);
}

// A pending slot turns into one card per mail once its thread arrives; the new
// elements need observing too.
watch(
  () => feed.value.length,
  async () => {
    await nextTick();
    observeCards();
  },
);

onBeforeUnmount(() => {
  cardObserver?.disconnect();
  hydration.reset();
});

watch(
  topicId,
  () => {
    confirmingDelete.value = false;
    editingTitle.value = false;
    filter.value = 'all';
    participantFilter.value = '';
    descriptionDraft.value = topic.value?.description ?? '';
    void loadTimeline(true);
  },
  { immediate: false },
);

watch(topic, (value) => {
  if (!editingTitle.value) descriptionDraft.value = value?.description ?? '';
});

onMounted(async () => {
  if (topics.items.length === 0) await topics.load();
  descriptionDraft.value = topic.value?.description ?? '';
  await loadTimeline(true);
});

async function startTitleEdit() {
  if (!topic.value) return;
  titleDraft.value = topic.value.name;
  editingTitle.value = true;
  await nextTick();
  titleInput.value?.focus();
  titleInput.value?.select();
}

async function saveTitle() {
  if (!editingTitle.value || !topic.value) return;
  editingTitle.value = false;
  const name = titleDraft.value.trim();
  if (!name || name === topic.value.name) return;
  try {
    await topics.upsert({ ...topic.value, name });
  } catch {
    toast.error(t('topics.toast.renameFailed'));
  }
}

async function startIconEdit() {
  if (!topic.value) return;
  iconDraft.value = topic.value.icon ?? '';
  editingIcon.value = true;
  await nextTick();
  iconInput.value?.focus();
  iconInput.value?.select();
}

/** Empty input clears the icon; the avatar then falls back to the newest sender. */
async function saveIcon() {
  if (!editingIcon.value || !topic.value) return;
  editingIcon.value = false;
  const icon = iconDraft.value.trim() || null;
  if (icon === topic.value.icon) return;
  try {
    await topics.upsert({ ...topic.value, icon });
  } catch {
    toast.error(t('topics.toast.iconFailed'));
  }
}

async function saveDescription() {
  if (!topic.value) return;
  const description = descriptionDraft.value.trim();
  if (description === topic.value.description) return;
  try {
    await topics.upsert({ ...topic.value, description });
  } catch {
    toast.error(t('topics.toast.descriptionFailed'));
  }
}

/** Done works like closing an issue: one button at the bottom, reversible. */
async function toggleDone() {
  if (!topic.value) return;
  const next = isDone.value ? 'active' : 'done';
  try {
    await topics.upsert({ ...topic.value, status: next });
    if (next === 'done') toast.success(t('topics.toast.done'));
    else toast.success(t('topics.toast.reopened'));
  } catch {
    toast.error(t('topics.toast.statusFailed'));
  }
}

async function deleteTopic() {
  if (!topic.value) return;
  if (!confirmingDelete.value) {
    confirmingDelete.value = true;
    return;
  }
  try {
    await topics.remove(topic.value.id);
    toast.success(t('topics.toast.deleted'));
    emit('deleted');
  } catch {
    toast.error(t('topics.toast.deleteFailed'));
  }
}

async function addNote() {
  const content = noteDraft.value.trim();
  if (!content || !topicId.value) return;
  try {
    await topics.upsertNote({ topicId: topicId.value, content });
    noteDraft.value = '';
    await loadTimeline();
    await nextTick();
    scrollToBottom();
  } catch {
    toast.error(t('topics.toast.noteSaveFailed'));
  }
}

async function saveNote(noteId: string, content: string) {
  try {
    await topics.upsertNote({ id: noteId, topicId: topicId.value, content });
    await loadTimeline();
  } catch {
    toast.error(t('topics.toast.noteSaveFailed'));
  }
}

async function deleteNote(noteId: string) {
  try {
    await topics.deleteNote(topicId.value, noteId);
    await loadTimeline();
  } catch {
    toast.error(t('topics.toast.noteDeleteFailed'));
  }
}

async function removeItem(item: TopicTimelineItem) {
  if (item.item_type === 'note') return;
  try {
    await topics.removeItemFromTopic(topicId.value, item.item_type, item.item_id);
    timeline.value = timeline.value.filter(
      (entry) => !(entry.item_type === item.item_type && entry.item_id === item.item_id),
    );
    toast.success(t('topics.toast.itemRemoved'), {
      action: {
        label: t('topics.toast.undo'),
        handler: () => {
          void topics
            .addItemsToTopic(topicId.value, item.item_type as 'thread' | 'message', [item.item_id])
            .then(() => loadTimeline());
        },
      },
    });
  } catch {
    toast.error(t('topics.toast.itemRemoveFailed'));
  }
}

function openThread(threadId: string) {
  emit('openThread', threadId);
}

/** Reply without leaving the topic — the composer opens on the thread. */
function replyToThread(threadId: string) {
  threadReply.reply(threadId);
}

// ── Wiedervorlage: a follow-up date, like snoozing a thread ────────────────
const duePickerOpen = ref(false);

const dueLabel = computed(() => {
  const value = topic.value?.due_date;
  if (!value) return '';
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString(locale.value, {
    day: '2-digit',
    month: '2-digit',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
});

const isDue = computed(() => {
  const value = topic.value?.due_date;
  return Boolean(value && value <= new Date().toISOString());
});

async function setDueDate(value: string | null) {
  if (!topic.value) return;
  duePickerOpen.value = false;
  try {
    await topics.upsert({ ...topic.value, due_date: value });
    toast.success(value ? t('topics.toast.dueSet') : t('topics.toast.dueCleared'));
  } catch {
    toast.error(t('topics.toast.dueFailed'));
  }
}

/** The whole topic as Markdown on the clipboard — notes, mails, attachments. */
async function copyAsMarkdown() {
  if (!topic.value) return;
  const lines: string[] = [`# ${topic.value.name}`, ''];
  if (topic.value.description) lines.push(topic.value.description, '');
  for (const entry of feed.value) {
    if (entry.item.item_type === 'note') {
      lines.push(
        `## ${t('topics.detail.markdown.note')} · ${entry.item.sort_date}`,
        '',
        entry.item.content ?? '',
        '',
      );
      continue;
    }
    const message = entry.message;
    if (!message) continue;
    lines.push(
      `## ${message.from.name || message.from.email} <${message.from.email}> · ${new Date(message.timestamp).toLocaleString(locale.value)}`,
      '',
      message.bodyText?.trim() || entry.item.snippet || '',
      '',
    );
  }
  if (allAttachments.value.length > 0) {
    lines.push(`## ${t('topics.detail.markdown.attachments')}`, '');
    for (const attachment of allAttachments.value) {
      lines.push(`- ${attachment.filename}`);
    }
    lines.push('');
  }
  try {
    await navigator.clipboard.writeText(lines.join('\n').trimEnd() + '\n');
    toast.success(t('topics.toast.markdownCopied'));
  } catch {
    toast.error(t('topics.toast.copyFailed'));
  }
}
</script>

<template>
  <section v-if="topic" class="topic-page">
    <CardButton v-show="scrolledAway" class="jump-bar" :title="$t('topics.detail.jumpToTop')" @click="scrollToTop">
      <span class="jump-bar__inner">
        <span class="state-dot" :class="isDone ? 'state-dot--done' : 'state-dot--open'" aria-hidden="true" />
        <strong class="jump-bar__name">{{ topic.name }}</strong>
        <span class="jump-bar__meta">
          {{ $t('topics.detail.entries', { count: topic.item_count + topic.note_count }) }}
          · {{ $t('topics.detail.attachmentsCount', { count: allAttachments.length + topicFiles.length }) }}
        </span>
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M3.5 10L8 5.5L12.5 10" />
        </svg>
      </span>
    </CardButton>
    <div ref="scrollEl" class="topic-scroll" @scroll.passive="onScroll">
    <div class="topic-issue">
      <!-- Issue head: title, state badge, meta -->
      <header class="issue-head">
        <div class="issue-title-row">
          <input
            v-if="editingIcon"
            ref="iconInput"
            v-model="iconDraft"
            class="icon-input"
            type="text"
            maxlength="4"
            :aria-label="$t('topics.detail.iconLabel')"
            placeholder="🙂"
            @keydown.enter.prevent="saveIcon"
            @keydown.esc="editingIcon = false"
            @blur="saveIcon"
          />
          <CardButton
            v-else
            class="icon-tile"
            :title="$t('topics.detail.iconTitle')"
            @click="startIconEdit"
          >
            <span v-if="topic.icon" class="icon-tile__emoji" aria-hidden="true">{{ topic.icon }}</span>
            <Avatar
              v-else
              :name="topic.last_sender_name || topic.last_sender_email || topic.name"
              :email="topic.last_sender_email"
              :size="34"
            />
          </CardButton>
          <input
            v-if="editingTitle"
            ref="titleInput"
            v-model="titleDraft"
            class="title-input"
            type="text"
            :aria-label="$t('topics.detail.nameLabel')"
            @keydown.enter.prevent="saveTitle"
            @keydown.esc="editingTitle = false"
            @blur="saveTitle"
          />
          <h1 v-else class="issue-title">
            {{ topic.name }}
            <IconButton size="sm" :title="$t('topics.detail.rename')" @click="startTitleEdit">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <path d="M12 1.5L14.5 4L5.5 13H3V10.5L12 1.5Z" />
              </svg>
            </IconButton>
          </h1>
          <span class="issue-spacer" />
          <span class="due-wrap">
            <IconButton
              size="md"
              :tone="isDue ? 'warning' : 'default'"
              :active="duePickerOpen"
              :title="topic.due_date ? $t('topics.detail.dueSet', { date: dueLabel }) : $t('topics.detail.dueAdd')"
              @click="duePickerOpen = !duePickerOpen"
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="8" cy="8.5" r="6" />
                <path d="M8 5.5V8.5L10.2 10" />
              </svg>
            </IconButton>
            <SnoozePicker :open="duePickerOpen" @choose="setDueDate($event)" @close="duePickerOpen = false" />
          </span>
          <IconButton size="md" :title="$t('topics.detail.copyMarkdown')" @click="void copyAsMarkdown()">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <rect x="2" y="2.5" width="12" height="11" rx="1.6" />
              <path d="M4.5 10.5V6L6.5 8.2L8.5 6V10.5M11.5 6V10.5M11.5 10.5L10 9M11.5 10.5L13 9" />
            </svg>
          </IconButton>
          <AppButton
            v-if="confirmingDelete"
            variant="danger"
            size="sm"
            @click="deleteTopic"
            @blur="confirmingDelete = false"
          >
            {{ $t('topics.detail.confirmDelete') }}
          </AppButton>
          <IconButton v-else size="md" tone="danger" :title="$t('topics.detail.delete')" @click="deleteTopic">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M2.5 4H13.5M6.5 4V2.5H9.5V4M4 4L4.8 13.5H11.2L12 4" />
            </svg>
          </IconButton>
        </div>
        <div class="issue-meta">
          <span class="state-badge" :class="isDone ? 'state-badge--done' : 'state-badge--open'">
            <svg v-if="isDone" width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M2.5 8.5L6.5 12.5L13.5 4" />
            </svg>
            <svg v-else width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="8" cy="8" r="5" />
            </svg>
            {{ isDone ? $t('common.done') : $t('topics.detail.stateOpen') }}
          </span>
          <span class="issue-count">{{ $t('topics.detail.entries', { count: topic.item_count + topic.note_count }) }}</span>
          <span v-if="topic.due_date" class="due-chip" :class="{ 'due-chip--due': isDue }">
            <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="8" cy="8.5" r="6" />
              <path d="M8 5.5V8.5L10.2 10" />
            </svg>
            {{ dueLabel }}
            <IconButton class="due-chip__clear" :title="$t('topics.detail.dueClear')" @click="void setDueDate(null)">×</IconButton>
          </span>
          <div v-if="participants.length > 1" class="participants" role="group" :aria-label="$t('topics.detail.participants')">
            <CardButton
              v-for="person in participants"
              :key="person.email"
              class="participants__avatar"
              :class="{ 'participants__avatar--active': participantFilter === person.email.toLowerCase() }"
              :title="participantFilter === person.email.toLowerCase()
                ? $t('topics.detail.filterParticipantOff', { name: person.name })
                : $t('topics.detail.filterParticipantOn', { name: person.name })"
              @click="toggleParticipant(person.email)"
            >
              <Avatar :name="person.name" :email="person.email" :size="24" />
            </CardButton>
          </div>
          <span class="issue-spacer" />
          <SegmentedControl
            :options="filterOptions"
            :model-value="filter"
            :aria-label="$t('topics.detail.filterLabel')"
            @update:model-value="filter = $event as 'all' | 'mail' | 'notes'"
          />
        </div>
        <textarea
          v-model="descriptionDraft"
          class="description"
          rows="2"
          :placeholder="$t('topics.detail.descriptionPlaceholder')"
          :aria-label="$t('topics.detail.descriptionLabel')"
          @blur="saveDescription"
        />

        <!-- Attachments: everything from the mails plus files dropped in directly -->
        <div class="attachments">
          <span class="attachments__label">
            {{ $t('topics.detail.attachments', { count: allAttachments.length + topicFiles.length }) }}
            <AppButton class="attachments__add" :disabled="uploadingFiles" @click="pickFiles" variant="link" size="sm">
              <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
                <path d="M8 3V13M3 8H13" />
              </svg>
              {{ $t('topics.detail.addFile') }}
            </AppButton>
            <input ref="fileInput" type="file" multiple class="attachments__input" :aria-label="$t('topics.detail.fileInputLabel')" @change="onFilesPicked" />
          </span>
          <div v-if="allAttachments.length > 0 || topicFiles.length > 0" class="attachments__list">
            <AttachmentChip
              v-for="attachment in allAttachments"
              :key="attachment.id"
              :attachment="attachment"
            />
            <div v-for="file in topicFiles" :key="file.id" class="topic-file">
              <div class="topic-file__meta">
                <span class="topic-file__name">{{ file.filename }}</span>
                <span class="topic-file__size">{{ formatBytes(file.size) }}</span>
              </div>
              <IconButton size="sm" :title="$t('topics.detail.saveFile')" @click="void downloadTopicFile(file)">
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M8 2.5V10.5M8 10.5L4.8 7.3M8 10.5L11.2 7.3" />
                  <path d="M3 13.5H13" />
                </svg>
              </IconButton>
              <IconButton size="sm" tone="danger" :title="$t('topics.detail.deleteFile')" @click="void deleteTopicFile(file)">
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M2.5 4H13.5M6.5 4V2.5H9.5V4M4 4L4.8 13.5H11.2L12 4" />
                </svg>
              </IconButton>
            </div>
          </div>
        </div>
      </header>

      <!-- Feed: oldest first, like an issue discussion -->
      <SkeletonRow v-if="timelineLoading && timeline.length === 0" :count="3" height="96px" />
      <div v-else-if="feed.length > 0" class="feed">
        <TopicFeedItem
          v-for="entry in feed"
          :key="entry.key"
          :data-thread-id="entry.item.thread_id || undefined"
          :item="entry.item"
          :message="entry.message"
          @open-thread="openThread"
          @reply="replyToThread"
          @remove="removeItem(entry.item)"
          @save-note="saveNote(entry.item.item_id, $event)"
          @delete-note="deleteNote(entry.item.item_id)"
        />
      </div>
      <EmptyState
        v-else
        :title="$t('topics.detail.emptyTitle')"
        :description="$t('topics.detail.emptyDescription')"
      >
        <template #icon>
      <svg width="44" height="44" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" opacity="0.55">
        <path d="M2 4.5C2 3.7 2.7 3 3.5 3H6L7.5 4.5H12.5C13.3 4.5 14 5.2 14 6V11.5C14 12.3 13.3 13 12.5 13H3.5C2.7 13 2 12.3 2 11.5V4.5Z" />
      </svg>
    </template>
      </EmptyState>

    </div>
    </div>

    <!-- Composer + close, pinned to the pane bottom like a chat input -->
    <div class="composer-bar">
      <form class="note-composer" @submit.prevent="addNote">
        <textarea
          v-model="noteDraft"
          rows="2"
          :placeholder="$t('topics.detail.notePlaceholder')"
          :aria-label="$t('topics.detail.noteLabel')"
          @keydown.enter.meta.prevent="addNote"
          @keydown.enter.ctrl.prevent="addNote"
        />
        <div class="note-composer__actions">
          <AppButton variant="ghost" size="sm" @click.prevent="toggleDone">
            {{ isDone ? $t('topics.detail.reopen') : $t('topics.detail.markDone') }}
          </AppButton>
          <AppButton variant="primary" size="sm" :disabled="!noteDraft.trim()" @click="addNote">
            {{ $t('topics.detail.addNote') }}
          </AppButton>
        </div>
      </form>
    </div>
  </section>

  <EmptyState
    v-else
    :title="$t('topics.detail.notFoundTitle')"
    :description="$t('topics.detail.notFoundDescription')"
    :action-label="$t('common.close')"
    @action="emit('deleted')"
  >
    <template #icon>
      <svg width="44" height="44" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" opacity="0.55">
        <path d="M2 4.5C2 3.7 2.7 3 3.5 3H6L7.5 4.5H12.5C13.3 4.5 14 5.2 14 6V11.5C14 12.3 13.3 13 12.5 13H3.5C2.7 13 2 12.3 2 11.5V4.5Z" />
      </svg>
    </template>
  </EmptyState>
</template>

<style scoped>
.topic-page {
  position: relative;
  height: 100%;
  display: grid;
  grid-template-rows: minmax(0, 1fr) auto;
  background: var(--bg-base);
}

/* Floating condensed head while the real one is scrolled away. */
.jump-bar {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  z-index: 5;
  border: 0;
  border-bottom: 1px solid var(--border-color-muted);
  background: color-mix(in srgb, var(--bg-base) 88%, transparent);
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
  font: inherit;
  padding: 8px 24px;
  cursor: pointer;
  text-align: left;
}

.jump-bar__inner {
  max-width: 860px;
  margin: 0 auto;
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  color: var(--text-secondary);
}

.jump-bar:hover .jump-bar__inner {
  color: var(--text-primary);
}

.state-dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  flex-shrink: 0;
}

.state-dot--open {
  background: var(--accent-green);
}

.state-dot--done {
  background: var(--accent-purple);
}

.jump-bar__name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-sm);
  font-weight: var(--weight-strong);
  color: var(--text-primary);
}

.jump-bar__meta {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.topic-scroll {
  min-height: 0;
  overflow-y: auto;
}

.topic-issue {
  max-width: 860px;
  margin: 0 auto;
  padding: 28px 24px 24px;
  display: grid;
  gap: 16px;
}

/* Attachments bundle in the issue head */
.attachments {
  display: grid;
  gap: 8px;
}

.attachments__label {
  font-size: var(--font-size-xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--text-tertiary);
}

.attachments__list {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.attachments__add { margin-left: 8px; }

.attachments__input {
  display: none;
}

.topic-file {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 6px 6px 12px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  max-width: 260px;
}

.topic-file__meta {
  min-width: 0;
  display: grid;
  gap: 1px;
  flex: 1;
}

.topic-file__name {
  font-size: var(--font-size-sm);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.topic-file__size {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

/* Sticky composer: always reachable, like a chat input */
.composer-bar {
  border-top: 1px solid var(--border-color-muted);
  background: var(--bg-base);
  padding: 10px 24px 14px;
}

.composer-bar .note-composer {
  max-width: 860px;
  margin: 0 auto;
}

.issue-head {
  display: grid;
  gap: 10px;
}

.issue-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.issue-title {
  margin: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.01em;
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.issue-spacer {
  flex: 1;
}

.title-input {
  flex: 1;
  min-width: 0;
  border: 1px solid var(--accent-primary);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-primary);
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  padding: 4px 10px;
}

.icon-tile {
  display: grid;
  place-items: center;
  width: 40px;
  height: 40px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--accent-purple) 12%, transparent);
  cursor: pointer;
  flex-shrink: 0;
  transition: background var(--duration-fast) var(--ease-out);
}

.icon-tile:hover {
  background: color-mix(in srgb, var(--accent-purple) 22%, transparent);
}

.icon-tile__emoji {
  font-size: 20px;
  line-height: 1;
}

.icon-input {
  width: 40px;
  height: 40px;
  border: 1px solid var(--accent-primary);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-primary);
  font-size: 18px;
  text-align: center;
  padding: 0;
  flex-shrink: 0;
}

.issue-meta {
  display: flex;
  align-items: center;
  gap: 10px;
}

/* GitHub-style state pill: green-ish while open, purple-ish when closed. */
.state-badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 12px;
  border-radius: var(--radius-full);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-on-accent);
}

.state-badge--open {
  background: var(--accent-green);
}

.state-badge--done {
  background: var(--accent-purple);
}

.issue-count {
  font-size: var(--font-size-sm);
  color: var(--text-tertiary);
}

.due-wrap {
  position: relative;
}

.due-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 8px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-primary) 10%, transparent);
  color: var(--accent-primary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
  font-variant-numeric: tabular-nums;
}

.due-chip--due {
  background: color-mix(in srgb, var(--accent-amber) 14%, transparent);
  color: var(--accent-amber);
}

.due-chip__clear {
  border: 0;
  background: transparent;
  color: inherit;
  font-size: var(--font-size-sm);
  line-height: 1;
  padding: 0 2px;
  cursor: pointer;
  opacity: 0.7;
}

.due-chip__clear:hover {
  opacity: 1;
}

.participants {
  display: flex;
  align-items: center;
  gap: 2px;
}

.participants__avatar {
  border: 2px solid transparent;
  border-radius: var(--radius-full);
  background: transparent;
  padding: 0;
  cursor: pointer;
  line-height: 0;
  transition: border-color var(--duration-fast) var(--ease-out), opacity var(--duration-fast) var(--ease-out);
}

.participants__avatar:hover {
  border-color: color-mix(in srgb, var(--accent-primary) 45%, transparent);
}

.participants__avatar--active {
  border-color: var(--accent-primary);
}

.description {
  width: 100%;
  resize: vertical;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-sm);
  padding: 10px 12px;
}

.description:focus {
  border-color: var(--accent-primary);
  color: var(--text-primary);
}

.feed {
  display: grid;
  gap: 12px;
}

.note-composer {
  display: grid;
  gap: 8px;
}

.note-composer textarea {
  width: 100%;
  resize: vertical;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
  padding: 10px 12px;
}

.note-composer textarea:focus {
  border-color: var(--accent-primary);
}

.note-composer__actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
