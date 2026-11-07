import { invoke } from './core';
import type { BackendOutgoingAttachment } from '@/types/composer';

/*
 * Mail dragged onto the composer, as `.eml` attachments. Apart from
 * `composer.ts` because the message viewer shares that module's chunk and
 * never attaches anything.
 */

/**
 * Original sources of the threads dropped on a composer, one `.eml` per
 * thread. Threads whose source cannot be fetched are missing from the result.
 */
export async function backendThreadEmlAttachments(
  threadIds: string[],
): Promise<BackendOutgoingAttachment[]> {
  return invoke<BackendOutgoingAttachment[]>('thread_eml_attachments', { threadIds });
}

/**
 * Original sources of single messages dropped on a composer (a search hit,
 * one message out of a thread), one `.eml` each. Drafts and messages whose
 * source cannot be fetched are missing from the result.
 */
export async function backendMessageEmlAttachments(
  messageIds: string[],
): Promise<BackendOutgoingAttachment[]> {
  return invoke<BackendOutgoingAttachment[]>('message_eml_attachments', { messageIds });
}
