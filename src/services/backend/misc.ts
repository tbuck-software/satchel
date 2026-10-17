import type { RefinedWireFields } from './wireTypes';
import type { SmartInboxPreviewDto as WireSmartInboxPreviewDto } from './generated/app/commands/threads/SmartInboxPreviewDto';
import type { SmartInboxBucketDto as WireSmartInboxBucketDto } from './generated/app/commands/threads/SmartInboxBucketDto';
import type { SmartInboxSummaryDto as WireSmartInboxSummaryDto } from './generated/app/commands/threads/SmartInboxSummaryDto';
import type { BatchTransitionInput as WireBatchTransitionInput } from './generated/app/commands/workflow/BatchTransitionInput';
import type { BatchTransitionDto as WireBatchTransitionDto } from './generated/app/commands/workflow/BatchTransitionDto';
import type { SenderPreference as WireSenderPreference } from './generated/app/commands/categories/SenderPreference';
import type { OptionalWireFields } from './wireTypes';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type { BackendRuntimeCapabilities } from './core';
import type { WorkflowState } from '@/types/workflow';
import type { BackendBlockedSender } from './system';

export type BackendSmartInboxPreview = WireSmartInboxPreviewDto;


export type BackendSmartInboxBucket = WireSmartInboxBucketDto;


export type BackendSmartInboxSummary = WireSmartInboxSummaryDto;


export type BatchTransitionInput = OptionalWireFields<WireBatchTransitionInput, 'snooze_until'>;


export type BatchTransitionResult = WireBatchTransitionDto;


export async function backendRecordFrontendDiagnostic(input: {
  event: string;
  detail: string;
  cacheThreads: number;
  cachedMessages: number;
  bodyChars: number;
  domNodes: number;
  iframeCount: number;
  renderedMailCount: number;
  jsHeapBytes: number;
  level?: 'info' | 'warn' | 'error';
}): Promise<void> {
  return invoke<void>('record_frontend_diagnostic', {
    input: {
      event: input.event,
      detail: input.detail,
      cache_threads: input.cacheThreads,
      cached_messages: input.cachedMessages,
      body_chars: input.bodyChars,
      dom_nodes: input.domNodes,
      iframe_count: input.iframeCount,
      rendered_mail_count: input.renderedMailCount,
      js_heap_bytes: input.jsHeapBytes,
      level: input.level ?? 'info',
    },
  });
}


export async function backendListSmartInboxSummary(
  accountId?: string | null,
): Promise<BackendSmartInboxSummary> {
  return invoke<BackendSmartInboxSummary>('list_smart_inbox_summary', {
    accountId: accountId ?? null,
  });
}


export async function backendSetSenderPreference(input: {
  accountId: string;
  senderEmail: string;
  category?: 'personal' | 'updates' | 'promotions' | 'social' | null;
  isPriority?: boolean | null;
}): Promise<void> {
  return invoke<void>('set_sender_preference', {
    input: {
      account_id: input.accountId,
      sender_email: input.senderEmail,
      ...(input.category !== undefined ? { category: input.category } : {}),
      ...(input.isPriority !== undefined ? { is_priority: input.isPriority } : {}),
    },
  });
}

export type BackendSenderPreference = RefinedWireFields<WireSenderPreference, { category: 'personal' | 'updates' | 'promotions' | 'social' | null }>;


/** Current filing of a sender, across all accounts. */
export async function backendGetSenderPreference(
  senderEmail: string,
): Promise<BackendSenderPreference> {
  return invoke<BackendSenderPreference>('get_sender_preference', { senderEmail });
}


/** One thread in list shape — for hits outside the loaded list (search). */

export async function backendListRecentTransitions(): Promise<
  Array<{ thread_id: string; from: WorkflowState; to: WorkflowState; at: string }>
> {
  return invoke('list_recent_transitions');
}


export async function backendBlockSender(input: {
  pattern: string;
  accountId?: string | null;
}): Promise<BackendBlockedSender> {
  return invoke<BackendBlockedSender>('block_sender', {
    input: { pattern: input.pattern, account_id: input.accountId ?? null },
  });
}


export async function backendUnblockSender(id: string): Promise<void> {
  return invoke<void>('unblock_sender', { id });
}


export async function backendRuntimeCapabilities(): Promise<BackendRuntimeCapabilities> {
  return invoke<BackendRuntimeCapabilities>('runtime_capabilities');
}

/**
 * Hold the process open on Android so IMAP IDLE keeps its connection.
 *
 * Android freezes a backgrounded app and the low-memory killer takes it minutes
 * later, and mail arriving after that is noticed by nobody. A foreground
 * service keeps the process out of the cached bucket. It costs a permanent,
 * silent entry in the notification shade — Android draws one for every
 * foreground service and gives apps no way to hide it.
 *
 * A no-op on desktop, where the process simply keeps running.
 */
