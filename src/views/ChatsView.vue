<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRoute, useRouter } from 'vue-router';
import ListDetailLayout from '@/components/layout/ListDetailLayout.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import ListRowButton from '@/components/ui/ListRowButton.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import ErrorState from '@/components/ui/ErrorState.vue';
import SkeletonRow from '@/components/ui/SkeletonRow.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CountBadge from '@/components/ui/CountBadge.vue';
import Avatar from '@/components/common/Avatar.vue';
import ChatConversation from '@/components/chats/ChatConversation.vue';
import { chatSnippet } from '@/components/chats/chatText';
import { useAccountsStore } from '@/stores/accounts';
import { useChatsStore } from '@/stores/chats';
import { useAliasesStore } from '@/stores/aliases';
import { useLabsStore } from '@/stores/labs';
import { useThreadsStore } from '@/stores/threads';
import { useUiStore } from '@/stores/ui';
import { registerListKeyboardNavigation } from '@/composables/useListKeyboardNavigation';
import { formatListTime } from '@/utils/listTime';
import type { Thread } from '@/types/mail';

/**
 * Labs → Chats: the threads whose messages carry `Chat-Version`, as a
 * messenger shows them. The mail stays where it is; this list is a view of it
 * (`mailbox_kind: "chat"` in `list_threads`), across all visible accounts.
 */
const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const ui = useUiStore();
const labs = useLabsStore();
const accounts = useAccountsStore();
const aliases = useAliasesStore();
const threadsStore = useThreadsStore();
const chatsStore = useChatsStore();

const chats = computed(() => chatsStore.items);
const loaded = computed(() => chatsStore.loaded);
const loadFailed = computed(() => chatsStore.loadFailed);
const focusIndex = ref(-1);
const listRef = ref<HTMLElement | null>(null);
const conversation = ref<InstanceType<typeof ChatConversation> | null>(null);

const selectedId = computed(() => (typeof route.query.thread === 'string' ? route.query.thread : ''));
const selected = computed(() => chats.value.find((chat) => chat.id === selectedId.value) ?? null);

const ownAddresses = computed(() => new Set([
  ...accounts.accounts.map((account) => account.email.trim().toLowerCase()),
  ...aliases.items.map((alias) => alias.email.trim().toLowerCase()),
]));

/**
 * The list's name for a chat. A group names itself in the subject (`Chat:
 * <group>: …`, spec.md "Groups"); the chat view reads the real group name.
 */
function chatTitle(chat: Thread): string {
  const group = /^(?:re:\s*)?chat:\s*([^:]+)/i.exec(chat.subject)?.[1]?.trim();
  if (group) return group;
  const others = chat.participants.filter((person) => !ownAddresses.value.has(person.email.toLowerCase()));
  return others.map((person) => person.name || person.email).join(', ') || chat.subject;
}

function chatEmail(chat: Thread): string | null {
  const others = chat.participants.filter((person) => !ownAddresses.value.has(person.email.toLowerCase()));
  return others.length === 1 ? others[0]!.email : null;
}

function load() {
  return chatsStore.load();
}

// New mail reaches the thread store first; follow it, a burst at a time.
let reloadTimer: ReturnType<typeof setTimeout> | null = null;
watch(() => threadsStore.threads, () => {
  if (reloadTimer) clearTimeout(reloadTimer);
  reloadTimer = setTimeout(() => { reloadTimer = null; void load(); }, 400);
});

onMounted(async () => {
  await labs.loadOnce();
  if (!labs.isEnabled('chats')) {
    void router.replace({ name: 'inbox' });
    return;
  }
  await load();
});

// Switched off in Settings while open: there is nothing to show any more.
watch(() => labs.isEnabled('chats'), (on) => {
  if (!on) void router.replace({ name: 'inbox' });
});

function select(id: string) {
  const query = { ...route.query, thread: id };
  // A phone steps into the chat, so Back returns to the list.
  if (ui.isSinglePane && !selectedId.value) void router.push({ name: 'chats', query });
  else void router.replace({ name: 'chats', query });
}

function closeChat() {
  if (ui.isSinglePane && window.history.length > 1) {
    router.back();
    return;
  }
  const { thread: _thread, ...query } = route.query;
  void router.replace({ name: 'chats', query });
}


// ── Keyboard: j/k between chats, Enter opens, Escape leaves ──
function focusRow(index: number) {
  focusIndex.value = index;
  void nextTick(() => listRef.value?.querySelectorAll<HTMLElement>('[data-chat-row]')[index]?.focus());
}

onBeforeUnmount(registerListKeyboardNavigation({
  move: (delta) => {
    const count = chats.value.length;
    if (count === 0) return false;
    const current = focusIndex.value >= 0 ? focusIndex.value : chats.value.findIndex((chat) => chat.id === selectedId.value);
    focusRow(current < 0 ? 0 : Math.min(count - 1, Math.max(0, current + delta)));
    return true;
  },
  open: () => {
    const chat = chats.value[focusIndex.value];
    if (!chat) return false;
    select(chat.id);
    void nextTick(() => conversation.value?.focusInput());
    return true;
  },
  back: () => {
    if (selectedId.value) {
      const index = chats.value.findIndex((chat) => chat.id === selectedId.value);
      closeChat();
      if (index >= 0) focusRow(index);
      return true;
    }
    void router.push({ name: 'inbox' });
    return true;
  },
  targets: () => {
    if (selectedId.value) return [selectedId.value];
    const chat = chats.value[focusIndex.value];
    return chat ? [chat.id] : [];
  },
}));

onBeforeUnmount(() => {
  if (reloadTimer) clearTimeout(reloadTimer);
});
</script>

<template>
  <ListDetailLayout :has-detail="Boolean(selected)">
    <template #list>
      <section class="chats-list" :aria-label="$t('chats.list')">
        <PaneHeader :title="$t('chats.title')" :subtitle="$t('chats.allAccounts')" />
        <div v-if="!loaded" class="chats-list__body"><SkeletonRow :count="4" /></div>
        <ErrorState
          v-else-if="loadFailed && chats.length === 0"
          :title="$t('chats.loadFailed')"
          @retry="load"
        />
        <EmptyState
          v-else-if="chats.length === 0"
          :title="$t('chats.empty.title')"
          :description="$t('chats.empty.description')"
        >
          <template #icon>
            <AppIcon size="lg"><path d="M2.5 3.5h11v7h-7l-4 3z" /></AppIcon>
          </template>
        </EmptyState>
        <ul v-else ref="listRef" class="chats-list__body" role="list">
          <li v-for="(chat, index) in chats" :key="chat.id">
            <ListRowButton
              class="chat-row"
              :class="{ 'chat-row--selected': chat.id === selectedId, 'chat-row--unread': chat.unreadCount > 0 }"
              :aria-current="chat.id === selectedId ? 'true' : undefined"
              data-chat-row
              @focus="focusIndex = index"
              @click="select(chat.id)"
            >
              <Avatar :name="chatTitle(chat)" :email="chatEmail(chat)" :size="40" />
              <span class="chat-row__copy">
                <span class="chat-row__name">{{ chatTitle(chat) }}</span>
                <span class="chat-row__snippet">{{ chatSnippet(chat.snippet) }}</span>
              </span>
              <span class="chat-row__meta">
                <span class="chat-row__time">{{ formatListTime(chat.lastMessageAt) }}</span>
                <CountBadge
                  v-if="chat.unreadCount > 0"
                  :value="chat.unreadCount"
                  tone="accent"
                  :aria-label="t('chats.unread', { count: chat.unreadCount })"
                />
              </span>
            </ListRowButton>
          </li>
        </ul>
      </section>
    </template>
    <template #detail>
      <ChatConversation
        v-if="selected"
        ref="conversation"
        :key="selected.id"
        :thread="selected"
        @close="closeChat"
      />
    </template>
    <template #empty>
      <EmptyState :title="$t('chats.select.title')" :description="$t('chats.select.description')">
        <template #icon>
          <AppIcon size="lg"><path d="M2.5 3.5h11v7h-7l-4 3z" /></AppIcon>
        </template>
      </EmptyState>
    </template>
  </ListDetailLayout>
</template>

<style scoped>
.chats-list {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  height: 100%;
  min-height: 0;
  background: var(--bg-list);
}

.chats-list__body {
  min-height: 0;
  margin: 0;
  padding: var(--space-2xs) var(--space-xs);
  overflow-y: auto;
  list-style: none;
}

.chat-row {
  display: grid;
  grid-template-columns: 40px minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-sm);
  min-height: calc(var(--touch-target) + var(--space-md));
  padding: var(--space-sm);
}

.chat-row--selected {
  background: var(--bg-selected);
  box-shadow: inset 2px 0 0 var(--selection-edge);
}

.chat-row__copy {
  display: grid;
  min-width: 0;
  gap: var(--space-2xs);
}

.chat-row__name,
.chat-row__snippet {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chat-row__name {
  color: var(--text-primary);
  font-weight: var(--weight-medium);
}

.chat-row--unread .chat-row__name {
  font-weight: var(--weight-bold);
}

.chat-row__snippet {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.chat-row__meta {
  display: grid;
  justify-items: end;
  gap: var(--space-2xs);
  align-self: start;
}

.chat-row__time {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>
