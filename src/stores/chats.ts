import { defineStore } from 'pinia';
import { ref } from 'vue';
import {
  backendCommandsAvailable,
  backendGetChatMeta,
  backendListThreads,
  backendSendChatMessage,
  backendUnlockChatKey,
  type BackendChatMeta,
  type BackendSendMessageResult,
} from '@/services/backend';
import { toThreadModel } from '@/utils/threadList';
import type { Thread } from '@/types/mail';
import { useAccountsStore } from './accounts';
import { orderedLoad } from './internal/orderedLoad';

export type ChatMeta = BackendChatMeta;

/** One page is the whole list: a person has dozens of chats, not thousands. */
const CHAT_PAGE = 200;

/**
 * Labs → Chats. The list is the thread list scoped to chat threads
 * (`mailbox_kind: "chat"`); the messages stay in the message store. Only the
 * Chats view imports this store, so nothing of it loads while the flag is off.
 */
export const useChatsStore = defineStore('chats', () => {
  const accounts = useAccountsStore();
  const items = ref<Thread[]>([]);
  const loaded = ref(false);
  const loader = orderedLoad();
  const { loading, loadFailed } = loader;

  async function load() {
    if (!backendCommandsAvailable()) {
      loaded.value = true;
      return;
    }
    await loader.load(
      () => backendListThreads({ mailbox_kind: 'chat', limit: CHAT_PAGE }),
      (page) => { items.value = page.threads.map((row) => toThreadModel(row, accounts.accounts)); },
    );
    loaded.value = true;
  }

  async function meta(threadId: string): Promise<ChatMeta> {
    return backendGetChatMeta(threadId);
  }

  async function send(threadId: string, text: string, allowUnsigned = false): Promise<BackendSendMessageResult> {
    const result = await loader.mutate(() => backendSendChatMessage(threadId, text, allowUnsigned));
    void load();
    return result;
  }

  /** Read locally the moment the chat is read; the thread action saves it. */
  function markRead(threadId: string) {
    const chat = items.value.find((item) => item.id === threadId);
    if (chat) chat.unreadCount = 0;
  }

  /** Unlock the own key for signing; without a passphrase, try what is stored. */
  async function unlockKey(keyId: string, passphrase?: string): Promise<void> {
    await backendUnlockChatKey(keyId, passphrase);
  }

  return { items, loaded, loading, loadFailed, load, meta, send, unlockKey, markRead };
});
