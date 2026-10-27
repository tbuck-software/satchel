// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type {
  BackendAttachmentDto,
  BackendDownloadAttachmentResult,
  BackendInlineCacheStatus,
  BackendLoadedMessageBody,
  BackendMessage,
  BackendMessageBody,
  BackendMessageSearchResponse,
  BackendOpenedThread,
  BackendSearchListScope,
  BackendServerSearchHit,
  BackendUnsubscribeResult,
} from '@/types/messages';

export type {
  BackendAttachmentDto,
  BackendDownloadAttachmentResult,
  BackendInlineCacheProgress,
  BackendInlineCacheStatus,
  BackendLoadedMessageBody,
  BackendMessage,
  BackendMessageBody,
  BackendMessageSearchHit,
  BackendMessageSearchResponse,
  BackendOpenedThread,
  BackendSearchListScope,
  BackendSearchWarning,
  BackendServerSearchHit,
  BackendUnsubscribeResult,
} from '@/types/messages';


export async function backendListMessagesPreview(threadId: string): Promise<BackendMessage[]> {
  return invoke<BackendMessage[]>('list_messages_preview', { threadId });
}


/**
 * Previews of a whole thread and the bodies of its latest `bodyCount`
 * messages (plus drafts) in one round trip — what the reader needs to paint.
 */
export async function backendLoadThreadMessages(
  threadId: string,
  bodyCount: number,
): Promise<BackendOpenedThread> {
  return invoke<BackendOpenedThread>('load_thread_messages', {
    input: { thread_id: threadId, body_count: bodyCount },
  });
}


export async function backendListMessageBodies(messageIds: string[]): Promise<BackendMessageBody[]> {
  return invoke<BackendMessageBody[]>('list_message_bodies', {
    input: {
      message_ids: messageIds,
    },
  });
}


export async function backendSearchMessageHits(
  query: string,
  options?: {
    accountId?: string;
    limit?: number;
    cursor?: NonNullable<BackendMessageSearchResponse['next_cursor']>;
    /** Only hits whose thread is in this list, as the list itself decides. */
    listScope?: BackendSearchListScope;
  },
): Promise<BackendMessageSearchResponse> {
  return invoke<BackendMessageSearchResponse>('search_message_hits', {
    input: {
      query,
      account_id: options?.accountId ?? null,
      limit: options?.limit ?? null,
      cursor: options?.cursor ?? null,
      list_scope: options?.listScope ?? null,
    },
  });
}


export async function backendUnsubscribe(messageId: string): Promise<BackendUnsubscribeResult> {
  return invoke<BackendUnsubscribeResult>('unsubscribe', {
    input: { message_id: messageId },
  });
}


/** Attachment rows for several messages, in one round trip. */
export async function backendListAttachmentsForMessages(
  messageIds: string[],
): Promise<BackendAttachmentDto[]> {
  return invoke<BackendAttachmentDto[]>('list_attachments_for_messages', { messageIds });
}


export async function backendDownloadAttachment(
  attachmentId: string,
): Promise<BackendDownloadAttachmentResult> {
  return invoke<BackendDownloadAttachmentResult>('download_attachment', {
    input: { attachment_id: attachmentId },
  });
}

/**
 * Open a prepared attachment with the operating system's registered app.
 *
 * `confirmedExecutable` is the answer to the dialog the backend demands for a
 * type the OS runs; without it those are refused rather than handed over.
 */
export async function backendOpenAttachmentDefault(
  attachmentId: string,
  confirmedExecutable = false,
): Promise<void> {
  return invoke<void>('open_attachment_default', {
    input: { attachment_id: attachmentId, confirmed_executable: confirmedExecutable },
  });
}

/** Toggle native Quick Look. Returns false on platforms without Quick Look. */
export async function backendToggleAttachmentQuickLook(attachmentId: string): Promise<boolean> {
  return invoke<boolean>('toggle_attachment_quick_look', {
    input: { attachment_id: attachmentId },
  });
}


export async function backendInlineImageCacheStatus(): Promise<BackendInlineCacheStatus> {
  return invoke<BackendInlineCacheStatus>('inline_image_cache_status');
}


export async function backendStartInlineImageBackfill(): Promise<void> {
  return invoke<void>('start_inline_image_backfill');
}


export async function backendStopInlineImageBackfill(): Promise<void> {
  return invoke<void>('stop_inline_image_backfill');
}


export async function backendFetchServerSearchHit(input: {
  accountId: string;
  remoteId: string;
}): Promise<{ message_id: string; thread_id: string }> {
  return invoke('fetch_server_search_hit', {
    input: { account_id: input.accountId, remote_id: input.remoteId },
  });
}


export async function backendServerSearch(input: {
  accountId: string;
  query: string;
}): Promise<BackendServerSearchHit[]> {
  return invoke<BackendServerSearchHit[]>('server_search', {
    input: { account_id: input.accountId, query: input.query },
  });
}


/**
 * Fetch a body the sync stored the message without.
 *
 * Envelopes are cheap and bodies are not, so a phone is meant to keep every
 * message's header and only recent bodies. This is what makes an older message
 * open anyway instead of showing a blank page.
 */
export async function backendLoadMessageBody(
  messageId: string,
): Promise<BackendLoadedMessageBody> {
  return invoke<BackendLoadedMessageBody>('load_message_body', {
    input: { message_id: messageId },
  });
}
