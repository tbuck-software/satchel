<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import ErrorState from '@/components/ui/ErrorState.vue';
import IconButton from '@/components/ui/IconButton.vue';
import SkeletonRow from '@/components/ui/SkeletonRow.vue';
import Avatar from '@/components/common/Avatar.vue';
import { useAccountsStore } from '@/stores/accounts';
import { useChatsStore, type ChatMeta, type ChatSendChoices } from '@/stores/chats';
import { useAliasesStore } from '@/stores/aliases';
import { useMessagesStore } from '@/stores/messages';
import { useUiStore } from '@/stores/ui';
import { useThreadActions } from '@/composables/useThreadActions';
import { useToast } from '@/composables/useToast';
import { useConfirm } from '@/composables/useConfirm';
import { usePrompt } from '@/composables/usePrompt';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { backendErrorKey } from '@/utils/backendErrors';
import type { Thread } from '@/types/mail';
import ChatComposer from './ChatComposer.vue';
import { chatDays, relativeDay } from './chatText';

/**
 * One chat: the thread's messages as bubbles, grouped by day, and the quick
 * reply under them. The messages come from the message store like the
 * reader's; this is a different view of the same thread, not a copy of it.
 */
const props = defineProps<{ thread: Thread }>();
const emit = defineEmits<{ close: [] }>();

const { t, d } = useI18n();
const router = useRouter();
const ui = useUiStore();
const accounts = useAccountsStore();
const aliases = useAliasesStore();
const messagesStore = useMessagesStore();
const chatsStore = useChatsStore();
const threadActions = useThreadActions();
const toast = useToast();
const { ask: confirm } = useConfirm();
const { ask: prompt } = usePrompt();

const meta = ref<ChatMeta | null>(null);
const metaFailed = ref(false);
const sending = ref(false);
const drafts = new Map<string, string>();
const draft = ref('');
const scroller = ref<HTMLElement | null>(null);
const composer = ref<InstanceType<typeof ChatComposer> | null>(null);

const threadId = computed(() => props.thread.id);
const messages = computed(() => messagesStore.getByThread(threadId.value));
const loading = computed(() => messagesStore.isLoading(threadId.value) && messages.value.length === 0);

const ownAddresses = computed(() => {
  const own = new Set<string>();
  const account = accounts.accounts.find((entry) => entry.id === props.thread.accountId);
  if (account?.email) own.add(account.email.trim().toLowerCase());
  for (const alias of aliases.items) {
    if (alias.account_id === props.thread.accountId) own.add(alias.email.trim().toLowerCase());
  }
  if (meta.value?.own_address) own.add(meta.value.own_address.toLowerCase());
  return own;
});

/**
 * Mail that threaded into the chat from neither you nor a member (the
 * backend's `untrusted_message_ids`): shown, but marked, and never as yours,
 * even when it claims your address.
 */
const untrusted = computed(() => new Set(meta.value?.untrusted_message_ids ?? []));
const days = computed(() => chatDays(messages.value, ownAddresses.value).map((day) => ({
  ...day,
  bubbles: day.bubbles.map((bubble) => untrusted.value.has(bubble.id)
    ? { ...bubble, own: false, continued: false, outsider: true }
    : { ...bubble, outsider: false }),
})));
const isGroup = computed(() => Boolean(meta.value?.group_id));
const members = computed(() => meta.value?.members ?? []);
const title = computed(() => {
  if (meta.value?.group_name) return meta.value.group_name;
  const peer = members.value[0];
  if (peer) return peer.name || peer.address;
  const other = props.thread.participants.find((person) => !ownAddresses.value.has(person.email.toLowerCase()));
  return other?.name || other?.email || props.thread.subject;
});
const subtitle = computed(() => {
  if (isGroup.value) return t('chats.members', members.value.length + 1);
  return members.value[0]?.address ?? '';
});
const encryptionHint = computed(() => {
  if (!meta.value || meta.value.encrypts) return '';
  return meta.value.own_key
    ? t('chats.notEncryptedNoPeerKey')
    : t('chats.notEncryptedNoKey', { address: meta.value.own_address });
});

function dayLabel(date: Date): string {
  const relative = relativeDay(date);
  if (relative === 'today') return t('chats.today');
  if (relative === 'yesterday') return t('chats.yesterday');
  return d(date, date.getFullYear() === new Date().getFullYear() ? 'long' : 'dateWithYear');
}

function scrollToEnd() {
  void nextTick(() => {
    const element = scroller.value;
    if (element) element.scrollTop = element.scrollHeight;
  });
}

async function loadMeta(id: string) {
  metaFailed.value = false;
  try {
    const next = await chatsStore.meta(id);
    if (id === threadId.value) meta.value = next;
  } catch {
    if (id === threadId.value) metaFailed.value = true;
  }
}

let release: (() => void) | null = null;
let markReadTimer: ReturnType<typeof setTimeout> | null = null;

async function open(id: string) {
  release?.();
  release = messagesStore.retainThread(id);
  meta.value = null;
  void loadMeta(id);
  await messagesStore.openThread(id);
  if (id !== threadId.value) return;
  // A chat shows every line, so every body is needed, not only the latest.
  const ids = messagesStore.getByThread(id).map((message) => message.id);
  scrollToEnd();
  await messagesStore.loadAttachmentsForMessages(id, ids);
  if (id === threadId.value) scrollToEnd();
  if (props.thread.unreadCount > 0) {
    // The same dwell as the reader: passing through with j/k is not reading.
    if (markReadTimer) clearTimeout(markReadTimer);
    markReadTimer = setTimeout(() => {
      markReadTimer = null;
      if (id !== threadId.value) return;
      chatsStore.markRead(id);
      void threadActions.setRead([id], true);
    }, 800);
  }
}

watch(threadId, (id, previous) => {
  if (previous) drafts.set(previous, draft.value);
  draft.value = drafts.get(id) ?? '';
  void open(id);
}, { immediate: true });

watch(() => messages.value.length, scrollToEnd);

onBeforeUnmount(() => {
  release?.();
  if (markReadTimer) clearTimeout(markReadTimer);
});

/**
 * Owner decision: an encrypted reply is signed. With the key locked, ask to
 * unlock it first; only when that is declined, offer to send encrypted but
 * unsigned. Answers whether to send unsigned, or `null` to keep the text.
 */
async function prepareSignature(): Promise<boolean | null> {
  const current = meta.value;
  if (!current?.encrypts || current.own_key_unlocked || !current.own_key_id) return false;
  const keyId = current.own_key_id;
  try {
    // A key without a passphrase, or one Satchel keeps, unlocks without a question.
    await chatsStore.unlockKey(keyId);
    void loadMeta(threadId.value);
    return false;
  } catch {
    // It needs the passphrase.
  }
  for (;;) {
    const passphrase = await prompt({
      title: t('chats.unlock.title'),
      body: t('chats.unlock.body', { address: current.own_address }),
      label: t('chats.unlock.label'),
      confirmLabel: t('chats.unlock.confirm'),
      inputType: 'password',
    });
    if (passphrase === null) break;
    try {
      await chatsStore.unlockKey(keyId, passphrase);
      void loadMeta(threadId.value);
      return false;
    } catch (error) {
      toast.error(backendErrorKey(error) === 'backend.errors.pgpPassphraseMismatch'
        ? t('chats.unlock.wrong')
        : t('chats.unlock.failed'));
    }
  }
  const unsigned = await confirm({
    title: t('chats.unsigned.title'),
    body: t('chats.unsigned.body'),
    confirmLabel: t('chats.unsigned.confirm'),
  });
  return unsigned ? true : null;
}

/**
 * Any mail can thread into a chat. A reply whose recipients differ from the
 * last one, or that would leave an encrypted chat in plain text, waits for a
 * yes, and goes only to the recipients shown. `null` keeps the text.
 */
async function confirmDelivery(): Promise<ChatSendChoices | null> {
  const current = meta.value;
  if (!current) return null;
  const recipients = current.members.map((member) => member.address);
  // Until you have replied in a chat, who it goes to is read from mail anyone
  // can send: the first reply always shows its recipients (ADR 0020).
  const askRecipients = current.recipients_changed || current.first_reply;
  if (askRecipients) {
    const firstReply = current.first_reply && !current.recipients_changed;
    const group = Boolean(current.group_id);
    const yes = await confirm({
      title: !firstReply
        ? t('chats.confirm.recipientsTitle')
        : group ? t('chats.confirm.firstGroupTitle') : t('chats.confirm.firstTitle'),
      body: !firstReply
        ? t('chats.confirm.recipientsBody')
        : group ? t('chats.confirm.firstGroupBody') : t('chats.confirm.firstBody'),
      detail: current.members.map((member) => member.name ? `${member.name} <${member.address}>` : member.address).join(', '),
      confirmLabel: t('chats.confirm.send'),
    });
    if (!yes) return null;
  }
  if (current.encryption_downgrade) {
    const yes = await confirm({
      title: t('chats.confirm.plainTitle'),
      body: t('chats.confirm.plainBody'),
      confirmLabel: t('chats.confirm.sendPlain'),
      tone: 'danger',
    });
    if (!yes) return null;
  }
  return {
    expectedRecipients: recipients,
    confirmRecipients: askRecipients,
    allowPlaintext: current.encryption_downgrade,
    allowUnsigned: false,
  };
}

/** Both questions, in order: who and how in the clear, then the key. */
async function prepareSend(): Promise<ChatSendChoices | null> {
  const choices = await confirmDelivery();
  if (!choices) return null;
  const unsigned = await prepareSignature();
  if (unsigned === null) return null;
  return { ...choices, allowUnsigned: unsigned };
}

const RETRY_ERRORS = new Set(['backend.errors.pgpSigningKeyUnavailable', 'backend.errors.chatConfirmationNeeded']);

async function send(text: string) {
  const id = threadId.value;
  sending.value = true;
  try {
    let choices = await prepareSend();
    if (!choices) return;
    let result;
    try {
      result = await chatsStore.send(id, text, choices);
    } catch (error) {
      // The chat changed since it was read (a new member, a key locked in
      // another window): read it again and ask again, once.
      if (!RETRY_ERRORS.has(backendErrorKey(error) ?? '')) throw error;
      await loadMeta(id);
      choices = await prepareSend();
      if (!choices) return;
      result = await chatsStore.send(id, text, choices);
    }
    drafts.delete(id);
    if (id === threadId.value) draft.value = '';
    if (result.status === 'queued' || result.status === 'delivery_unknown') {
      toast.info(t('chats.composer.queued'));
    }
    await messagesStore.refreshThread(id);
    scrollToEnd();
  } catch (error) {
    toast.error(`${t('chats.composer.sendFailed')} ${backendErrorLabel(error)}`);
  } finally {
    sending.value = false;
  }
}

function showAsMail() {
  void router.push({ name: 'thread', params: { id: threadId.value } });
}

defineExpose({ focusInput: () => composer.value?.focus() });
</script>

<template>
  <section class="chat-conversation" :aria-label="title">
    <header class="chat-conversation__header">
      <IconButton
        v-if="ui.isSinglePane"
        :title="$t('common.back')"
        size="lg"
        @click="emit('close')"
      >
        <AppIcon size="lg"><path d="m10.5 3-5 5 5 5" /></AppIcon>
      </IconButton>
      <Avatar :name="title" :email="isGroup ? null : members[0]?.address" :size="36" />
      <div class="chat-conversation__heading">
        <h2>{{ title }}</h2>
        <p class="chat-conversation__subtitle">{{ subtitle }}</p>
        <p
          v-if="meta"
          class="chat-conversation__security"
          :class="{ 'chat-conversation__security--on': meta.encrypts }"
          :title="encryptionHint || undefined"
        >
          <AppIcon size="sm">
            <rect x="3.5" y="7" width="9" height="6.5" rx="1.5" />
            <path v-if="meta.encrypts" d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
            <path v-else d="M5.5 7V5a2.5 2.5 0 0 1 4.8-1" />
          </AppIcon>
          {{ meta.encrypts ? $t('chats.encrypted') : $t('chats.notEncrypted') }}
        </p>
      </div>
      <AppButton variant="secondary" size="sm" class="chat-conversation__as-mail" @click="showAsMail">
        {{ $t('chats.showAsMail') }}
      </AppButton>
    </header>

    <p v-if="encryptionHint" class="chat-conversation__hint">{{ encryptionHint }}</p>

    <div ref="scroller" class="chat-conversation__scroll" role="log" :aria-busy="loading">
      <div v-if="loading" class="chat-conversation__loading">
        <SkeletonRow :count="4" />
      </div>
      <ErrorState
        v-else-if="metaFailed && messages.length === 0"
        :title="$t('chats.messagesLoadFailed')"
        @retry="open(threadId)"
      />
      <template v-else>
        <section v-for="day in days" :key="day.key" class="chat-day" :aria-label="dayLabel(day.date)">
          <p class="chat-day__label">{{ dayLabel(day.date) }}</p>
          <div
            v-for="bubble in day.bubbles"
            :key="bubble.id"
            class="chat-bubble"
            :class="{ 'chat-bubble--own': bubble.own, 'chat-bubble--continued': bubble.continued }"
            :data-chat-bubble="bubble.own ? 'own' : 'peer'"
          >
            <p v-if="bubble.outsider" class="chat-bubble__sender chat-bubble__sender--outsider" data-chat-outsider>
              {{ $t('chats.notMember', { name: bubble.senderName }) }}
            </p>
            <p v-else-if="isGroup && !bubble.own && !bubble.continued" class="chat-bubble__sender">{{ bubble.senderName }}</p>
            <p class="chat-bubble__text">{{ bubble.text }}</p>
            <AppButton
              v-if="bubble.cut"
              variant="link"
              size="sm"
              class="chat-bubble__hidden"
              @click="showAsMail"
            >
              {{ $t('chats.partHidden') }}
            </AppButton>
            <time class="chat-bubble__time" :datetime="bubble.timestamp">{{ $d(new Date(bubble.timestamp), 'time') }}</time>
          </div>
        </section>
      </template>
    </div>

    <ChatComposer
      ref="composer"
      v-model="draft"
      :recipient="title"
      :sending="sending"
      @send="send"
    />
  </section>
</template>

<style scoped>
.chat-conversation {
  /* A column, not a grid: the notice row comes and goes, and the messages
     take whatever height the header, notice and reply field leave. */
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  min-height: 0;
  background: var(--bg-base);
}

.chat-conversation__header,
.chat-conversation__hint,
.chat-conversation > .chat-composer {
  flex: none;
}

.chat-conversation__header {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  min-width: 0;
  padding: var(--space-sm) var(--space-md);
  border-bottom: 1px solid var(--border-color-muted);
}

.chat-conversation__heading {
  flex: 1;
  min-width: 0;
}

.chat-conversation__heading h2 {
  margin: 0;
  overflow: hidden;
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chat-conversation__subtitle,
.chat-conversation__security {
  margin: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chat-conversation__security {
  display: flex;
  align-items: center;
  gap: var(--space-2xs);
  color: var(--text-tertiary);
  /* Whether the chat is encrypted is never cut off; it wraps instead. */
  white-space: normal;
}

.chat-conversation__security--on {
  color: var(--accent-green);
}

.chat-conversation__as-mail {
  flex: none;
}

.chat-conversation__hint {
  margin: 0;
  padding: var(--space-xs) var(--space-md);
  border-bottom: 1px solid var(--border-color-muted);
  background: var(--bg-surface);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.chat-conversation__scroll {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-md);
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-md);
  overscroll-behavior: contain;
}

.chat-conversation__loading {
  display: grid;
  gap: var(--space-sm);
}

.chat-day {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
}

.chat-day__label {
  align-self: center;
  margin: 0 0 var(--space-2xs);
  padding: var(--space-2xs) var(--space-sm);
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-size: var(--font-size-2xs);
}

.chat-bubble {
  align-self: flex-start;
  max-width: min(78%, 560px);
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--radius-lg);
  border-bottom-left-radius: var(--radius-xs);
  background: var(--bg-raised);
  color: var(--text-primary);
}

.chat-bubble--continued {
  margin-top: calc(-1 * var(--space-2xs));
}

.chat-bubble--own {
  align-self: flex-end;
  border-bottom-left-radius: var(--radius-lg);
  border-bottom-right-radius: var(--radius-xs);
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.chat-bubble__sender {
  margin: 0 0 var(--space-2xs);
  color: var(--text-accent);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
}

.chat-bubble__text {
  margin: 0;
  font-size: var(--font-size-md);
  line-height: 1.45;
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}

.chat-bubble__sender--outsider {
  color: var(--accent-red);
}

.chat-bubble__hidden {
  display: block;
  padding: 0;
  color: inherit;
  font-size: var(--font-size-xs);
  opacity: 0.8;
  text-decoration: underline;
}

.chat-bubble__time {
  display: block;
  margin-top: var(--space-2xs);
  font-size: var(--font-size-2xs);
  font-variant-numeric: tabular-nums;
  opacity: 0.75;
  text-align: right;
}

/* A phone gives the text the width; the header keeps its button with a label. */
@media (max-width: 767px) {
  .chat-bubble {
    max-width: 86%;
  }
}
</style>
