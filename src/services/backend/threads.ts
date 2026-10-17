import type { ProjectedWireFields, RefinedWireFields } from './wireTypes';
import type { ThreadDto as WireThreadDto } from './generated/app/commands/threads/ThreadDto';
import type { ThreadCursor as WireThreadCursor } from './generated/app/commands/threads/ThreadCursor';
import type { ListThreadsInput as WireListThreadsInput } from './generated/app/commands/threads/ListThreadsInput';
import type { ThreadPage as WireThreadPage } from './generated/app/commands/threads/ThreadPage';
import type { MessageLinkTargetDto as WireMessageLinkTargetDto } from './generated/app/commands/threads/MessageLinkTargetDto';
import type { RuleDto as WireRuleDto } from './generated/app/commands/rules/RuleDto';
import type { OptionalWireFields } from './wireTypes';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type { WorkflowState } from '@/types/workflow';
import type { ThreadSmartBucket } from '@/types/mail';
import type { BatchTransitionInput, BatchTransitionResult } from './misc';

export type BackendThread = RefinedWireFields<OptionalWireFields<WireThreadDto, 'sort_date' | 'has_draft' | 'draft_snippet' | 'board_position' | 'tags' | 'assignee' | 'shared_with' | 'sent_participants' | 'category' | 'smart_bucket' | 'is_muted' | 'is_starred' | 'snooze_until' | 'sender_known' | 'has_invite' | 'invite_status' | 'invite_cancelled'>, { smart_bucket?: ThreadSmartBucket }>;


export type BackendThreadCursor = WireThreadCursor;


export type BackendListThreadsInput = OptionalWireFields<WireListThreadsInput, 'account_id' | 'contact_email' | 'mailbox_kind' | 'mailbox_value' | 'workflow_state' | 'cursor' | 'before_cursor' | 'limit' | 'smart_bucket'>;


export type BackendThreadPage = ProjectedWireFields<WireThreadPage, { threads: BackendThread[] }>;


export async function backendListThreads(
  input?: BackendListThreadsInput,
): Promise<BackendThreadPage> {
  return invoke<BackendThreadPage>('list_threads', input ? { input } : {});
}


export async function backendListCachedThreads(
  input?: BackendListThreadsInput,
): Promise<BackendThreadPage | null> {
  return invoke<BackendThreadPage | null>('list_cached_threads', input ? { input } : {});
}


export async function backendSetThreadPriority(threadId: string, priority: number): Promise<void> {
  return invoke<void>('set_thread_priority', {
    input: { thread_id: threadId, priority },
  });
}


/** One thread in list shape — for hits outside the loaded list (search). */
export async function backendGetThread(threadId: string): Promise<BackendThread> {
  return invoke<BackendThread>('get_thread', { threadId });
}


/** Cached row revalidation; missing IDs are omitted. */
export async function backendGetThreads(threadIds: string[]): Promise<BackendThread[]> {
  return invoke<BackendThread[]>('get_threads', { threadIds });
}

/** What a `satchel://` link to this thread is built from. */
export type BackendMessageLinkTarget = WireMessageLinkTargetDto;

export async function backendMessageLinkTarget(threadId: string): Promise<BackendMessageLinkTarget> {
  return invoke<BackendMessageLinkTarget>('message_link_target', { threadId });
}

/** The thread holding this Message-ID, or null if Satchel does not have the mail. */
export async function backendResolveMessageLink(messageId: string): Promise<string | null> {
  return invoke<string | null>('resolve_message_link', { messageId });
}


export async function backendUpdateThreadState(
  threadId: string,
  nextState: WorkflowState,
  snoozeUntil?: string | null,
): Promise<BackendThread> {
  return invoke<BackendThread>('update_thread_state', {
    input: {
      thread_id: threadId,
      next_state: nextState,
      snooze_until: snoozeUntil ?? null,
    },
  });
}


export type BackendRule = ProjectedWireFields<OptionalWireFields<WireRuleDto, 'description'>, { trigger_config: Record<string, unknown> }>;


export type BackendRuleInput = Omit<BackendRule, 'id'> & { id?: string | null };


export async function backendListRules(): Promise<BackendRule[]> {
  return invoke<BackendRule[]>('list_rules');
}


export async function backendUpsertRule(input: BackendRuleInput): Promise<BackendRule> {
  return invoke<BackendRule>('upsert_rule', { input });
}


export async function backendDeleteRule(ruleId: string): Promise<void> {
  return invoke<void>('delete_rule', { ruleId });
}


export async function backendRunRulesForThread(threadId: string): Promise<string[]> {
  return invoke<string[]>('run_rules_for_thread', { threadId });
}


export async function backendRunRulesForThreads(threadIds: string[]): Promise<string[]> {
  return invoke<string[]>('run_rules_for_threads', { input: { thread_ids: threadIds } });
}


export async function backendUpdateThreadOrganization(input: {
  threadId: string;
  boardPosition?: number | null;
  assignee?: string | null;
  sharedWith?: string[];
}): Promise<void> {
  return invoke<void>('update_thread_organization', {
    input: {
      thread_id: input.threadId,
      board_position: input.boardPosition ?? null,
      assignee: input.assignee ?? null,
      shared_with: input.sharedWith ?? [],
    },
  });
}


export async function backendSetThreadTags(threadIds: string[], tags: string[]): Promise<void> {
  return invoke<void>('set_thread_tags', {
    input: { thread_ids: threadIds, tags },
  });
}


export async function backendTrashThreads(threadIds: string[]): Promise<void> {
  return invoke<void>('trash_threads', {
    input: {
      thread_ids: threadIds,
    },
  });
}


export async function backendSetThreadsRead(threadIds: string[], isRead: boolean): Promise<void> {
  return invoke<void>('set_threads_read', {
    input: {
      thread_ids: threadIds,
      is_read: isRead,
    },
  });
}


/** One round trip for a whole selection; each thread keeps its own `from`. */
export async function backendTransitionThreads(
  input: BatchTransitionInput,
): Promise<BatchTransitionResult> {
  return invoke('transition_threads', { input });
}


export async function backendSetThreadCategory(
  threadId: string,
  category: string | null,
): Promise<void> {
  return invoke<void>('set_thread_category', { threadId, category });
}


export async function backendSetThreadMuted(threadId: string, muted: boolean): Promise<void> {
  return invoke<void>('set_thread_muted', { input: { thread_id: threadId, muted } });
}


export async function backendMarkThreadsSpam(threadIds: string[]): Promise<void> {
  return invoke<void>('mark_threads_spam', { input: { thread_ids: threadIds } });
}


/** One round trip for a whole selection; the command flags them in one transaction. */
export async function backendSetThreadsStarred(
  threadIds: string[],
  starred: boolean,
): Promise<void> {
  return invoke<void>('set_threads_starred', {
    input: { thread_ids: threadIds, starred },
  });
}
