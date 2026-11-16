import type { ChatMetaDto } from './generated/app/commands/chats/ChatMetaDto';
import type { ChatMemberDto } from './generated/app/commands/chats/ChatMemberDto';
import type { SendChatMessageInput } from './generated/app/commands/chats/SendChatMessageInput';
import type { BackendSendMessageResult } from '@/types/composer';
import { invoke } from './core';
import { backendUnlockPgpKeys } from './security';

/** Group, members and encryption of a Delta Chat thread (Labs → Chats). */
export type BackendChatMeta = ChatMetaDto;
export type BackendChatMember = ChatMemberDto;

export async function backendGetChatMeta(threadId: string): Promise<BackendChatMeta> {
  return invoke<BackendChatMeta>('get_chat_meta', { threadId });
}

/** What the sender confirmed for one chat reply (see `SendChatMessageInput`). */
export type BackendChatSendConfirmation = Omit<SendChatMessageInput, 'thread_id' | 'text'>;

/**
 * Send a chat line now, through the outbox, as a Delta Chat message, to the
 * recipients the sender saw and with what they confirmed.
 */
export async function backendSendChatMessage(
  threadId: string,
  text: string,
  confirmation: BackendChatSendConfirmation,
): Promise<BackendSendMessageResult> {
  return invoke<BackendSendMessageResult>('send_chat_message', {
    input: { thread_id: threadId, text, ...confirmation },
  });
}

/** Unlock the key an encrypted chat reply is signed with. */
export async function backendUnlockChatKey(keyId: string, passphrase?: string): Promise<void> {
  await backendUnlockPgpKeys({ keyId, passphrase });
}
