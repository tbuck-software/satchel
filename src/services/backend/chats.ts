import type { ChatMetaDto } from './generated/app/commands/chats/ChatMetaDto';
import type { ChatMemberDto } from './generated/app/commands/chats/ChatMemberDto';
import type { BackendSendMessageResult } from '@/types/composer';
import { invoke } from './core';
import { backendUnlockPgpKeys } from './security';

/** Group, members and encryption of a Delta Chat thread (Labs → Chats). */
export type BackendChatMeta = ChatMetaDto;
export type BackendChatMember = ChatMemberDto;

export async function backendGetChatMeta(threadId: string): Promise<BackendChatMeta> {
  return invoke<BackendChatMeta>('get_chat_meta', { threadId });
}

/**
 * Send a chat line now, through the outbox, as a Delta Chat message.
 * `allowUnsigned` only after the sender declined to unlock their key and
 * chose to send encrypted without a signature.
 */
export async function backendSendChatMessage(
  threadId: string,
  text: string,
  allowUnsigned = false,
): Promise<BackendSendMessageResult> {
  return invoke<BackendSendMessageResult>('send_chat_message', {
    input: { thread_id: threadId, text, allow_unsigned: allowUnsigned },
  });
}

/** Unlock the key an encrypted chat reply is signed with. */
export async function backendUnlockChatKey(keyId: string, passphrase?: string): Promise<void> {
  await backendUnlockPgpKeys({ keyId, passphrase });
}
