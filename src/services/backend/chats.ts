import type { ChatMetaDto } from './generated/app/commands/chats/ChatMetaDto';
import type { ChatMemberDto } from './generated/app/commands/chats/ChatMemberDto';
import type { BackendSendMessageResult } from '@/types/composer';
import { invoke } from './core';

/** Group, members and encryption of a Delta Chat thread (Labs → Chats). */
export type BackendChatMeta = ChatMetaDto;
export type BackendChatMember = ChatMemberDto;

export async function backendGetChatMeta(threadId: string): Promise<BackendChatMeta> {
  return invoke<BackendChatMeta>('get_chat_meta', { threadId });
}

/** Send a chat line now, through the outbox, as a Delta Chat message. */
export async function backendSendChatMessage(threadId: string, text: string): Promise<BackendSendMessageResult> {
  return invoke<BackendSendMessageResult>('send_chat_message', { input: { thread_id: threadId, text } });
}
