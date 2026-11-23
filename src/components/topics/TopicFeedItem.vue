<script setup lang="ts">
import { computed, ref } from 'vue';
import { AppWindow, ExternalLink, Paperclip, Pencil, Reply, Trash2, X } from 'lucide';
import { useI18n } from 'vue-i18n';
import type { TopicTimelineItem } from '@/stores/topics';
import type { Message } from '@/types/mail';
import { mailGist, trimMailBodyHtml, trimMailBodyText } from '@/utils/mailGist';
import { isListWorthyAttachment } from '@/utils/attachments';
import Avatar from '@/components/common/Avatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import EmailRenderer from '@/components/mail/EmailRenderer.vue';

const props = defineProps<{
  item: TopicTimelineItem;
  /** Set for mail entries: exactly one message per feed card. */
  message?: Message;
}>();

const emit = defineEmits<{
  openThread: [threadId: string];
  reply: [threadId: string];
  remove: [];
  saveNote: [content: string];
  deleteNote: [];
}>();

const { locale } = useI18n();

const realAttachmentCount = computed(
  () => props.message?.attachments.filter(isListWorthyAttachment).length ?? 0,
);

const editing = ref(false);
const draft = ref('');
/** Keep the sender's formatting by default; the compact gist remains available on demand. */
const showOriginal = ref(true);

const formattedBody = computed(() => ({
  html: props.message?.bodyHtml ? trimMailBodyHtml(props.message.bodyHtml) : undefined,
  text: props.message?.bodyText ? trimMailBodyText(props.message.bodyText) : undefined,
}));

/** The mail as a comment: pleasantries, signature and quotes stripped. */
const gist = computed(() => {
  if (!props.message) return props.item.snippet ?? '';
  const text = props.message.bodyText?.trim();
  if (text) return mailGist(text);
  // HTML-only mails: strip the markup for the comment view.
  if (props.message.bodyHtml) {
    const stripped =
      new DOMParser().parseFromString(props.message.bodyHtml, 'text/html').body.textContent?.trim() ?? '';
    if (stripped) return mailGist(stripped);
  }
  return mailGist(props.item.snippet || '');
});

function startEdit() {
  draft.value = props.item.content ?? '';
  editing.value = true;
}

function saveEdit() {
  const content = draft.value.trim();
  if (!content) return;
  emit('saveNote', content);
  editing.value = false;
}

function formatDate(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString(locale.value, {
    day: '2-digit',
    month: '2-digit',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}
</script>

<template>
  <!-- Note: a comment card, like an issue comment. -->
  <article v-if="item.item_type === 'note'" class="card card--note">
    <header class="card__head">
      <span class="card__kind">{{ $t('topics.feed.note') }}</span>
      <time class="card__date">{{ formatDate(item.sort_date) }}</time>
      <span class="card__spacer" />
      <template v-if="!editing">
        <IconButton size="sm" :title="$t('topics.feed.editNote')" @click="startEdit">
          <AppIcon size="sm" :icon="Pencil" />
        </IconButton>
        <IconButton size="sm" tone="danger" :title="$t('topics.feed.deleteNote')" @click="emit('deleteNote')">
          <AppIcon size="sm" :icon="Trash2" />
        </IconButton>
      </template>
    </header>
    <div class="card__body">
      <template v-if="editing">
        <textarea v-model="draft" class="note-editor" rows="3" :placeholder="$t('topics.feed.notePlaceholder')" />
        <div class="note-editor-actions">
          <AppButton variant="primary" size="sm" @click="saveEdit">{{ $t('common.save') }}</AppButton>
          <AppButton variant="ghost" size="sm" @click="editing = false">{{ $t('common.cancel') }}</AppButton>
        </div>
      </template>
      <p v-else class="note-content">{{ item.content }}</p>
    </div>
  </article>

  <!-- One mail = one comment. The original body keeps the sender's formatting without nesting another mail card. -->
  <article v-else-if="message" class="card card--mail">
    <header class="card__head">
      <Avatar :name="message.from.name || message.from.email" :email="message.from.email" :size="24" />
      <span class="card__sender">
        <strong class="card__sender-name">{{ message.from.name || message.from.email }}</strong>
        <span v-if="message.from.name" class="card__sender-mail">{{ message.from.email }}</span>
      </span>
      <time class="card__date">{{ formatDate(message.timestamp) }}</time>
      <span class="card__spacer" />
      <IconButton
        v-if="item.thread_id"
        size="sm"
        :title="$t('common.reply')"
        @click="emit('reply', item.thread_id)"
      >
        <AppIcon size="sm" :icon="Reply" />
      </IconButton>
      <IconButton
        size="sm"
        :title="showOriginal ? $t('topics.feed.showGist') : $t('topics.feed.showOriginal')"
        :active="showOriginal"
        @click="showOriginal = !showOriginal"
      >
        <AppIcon size="sm" :icon="AppWindow" />
      </IconButton>
      <IconButton
        v-if="item.thread_id"
        size="sm"
        :title="$t('topics.feed.openInInbox')"
        @click="emit('openThread', item.thread_id)"
      >
        <AppIcon size="sm" :icon="ExternalLink" />
      </IconButton>
      <IconButton
        size="sm"
        :title="item.item_type === 'thread' ? $t('topics.feed.removeThread') : $t('topics.feed.removeMessage')"
        @click="emit('remove')"
      >
        <AppIcon size="sm" :icon="X" />
      </IconButton>
    </header>
    <div class="card__body">
      <EmailRenderer
        v-if="showOriginal"
        class="card__mail-body"
        :html="formattedBody.html"
        :text="formattedBody.text"
        :pgp-state="message.pgp?.state"
        :sender-email="message.from.email"
        :attachments="message.attachments"
        :normalize-quotes="true"
        :show-view-mode-toggle="false"
      />
      <template v-else>
        <p class="mail-gist">{{ gist || $t('topics.feed.noTextContent') }}</p>
      </template>
      <p v-if="realAttachmentCount > 0" class="mail-attachments">
        <AppIcon size="sm" :icon="Paperclip" />
        {{ $t('topics.feed.attachments', { count: realAttachmentCount }) }}
      </p>
    </div>
  </article>

  <!-- Messages still loading: keep the slot in the feed with the snippet. -->
  <article v-else class="card card--mail">
    <header class="card__head">
      <span class="card__kind">{{ item.item_type === 'thread' ? $t('topics.feed.thread') : $t('topics.feed.message') }}</span>
      <strong class="card__title">{{ item.title || $t('topics.feed.noSubject') }}</strong>
      <time class="card__date">{{ formatDate(item.sort_date) }}</time>
      <span class="card__spacer" />
      <IconButton size="sm" :title="$t('topics.feed.remove')" @click="emit('remove')">
        <AppIcon size="sm" :icon="X" />
      </IconButton>
    </header>
    <div class="card__body">
      <p class="mail-gist">{{ item.snippet || $t('common.loading') }}</p>
    </div>
  </article>
</template>

<style scoped>
.card {
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  overflow: hidden;
}

.card--note {
  background: color-mix(in srgb, var(--accent-amber) 5%, var(--bg-surface));
}

.card__head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  padding: 7px 12px;
  border-bottom: 1px solid var(--border-color-muted);
  background: color-mix(in srgb, var(--bg-raised) 55%, transparent);
}

.card__kind {
  font-size: var(--font-size-xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-tertiary);
  flex-shrink: 0;
}

.card__sender {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}

.card__sender-name {
  font-size: var(--font-size-sm);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.card__sender-mail {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.card__title {
  font-size: var(--font-size-sm);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

.card__date {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}

.card__spacer {
  flex: 1;
}

.card__body {
  padding: 10px 12px;
}

/* The card's side padding is the inset a phone's flush document steps out of
   (see EmailRenderer): designed mail reaches the card's edges. */
.card__mail-body {
  --reader-inline-inset: var(--space-md);
}

.card__mail-body :deep(.html-document-shell) {
  border: 0;
  border-radius: 0;
}

.mail-gist {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  line-height: 1.55;
  overflow-wrap: anywhere;
}

.mail-attachments {
  display: flex;
  align-items: center;
  gap: 5px;
  margin: 6px 0 0;
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.note-content {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  line-height: 1.55;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.note-editor {
  width: 100%;
  resize: vertical;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
  padding: 8px 10px;
}

.note-editor:focus {
  border-color: var(--accent-primary);
}

.note-editor-actions {
  display: flex;
  gap: 6px;
  margin-top: 8px;
}
</style>
