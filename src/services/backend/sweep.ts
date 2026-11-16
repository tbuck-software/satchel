import type { CleanupSender as WireCleanupSender } from './generated/core/db/sender_sweep/CleanupSender';
import type { SweepRuleRecord as WireSweepRuleRecord } from './generated/core/db/sender_sweep/SweepRuleRecord';
import type { SweepTarget as WireSweepTarget } from './generated/core/db/sender_sweep/SweepTarget';
import type { SweepSummary as WireSweepSummary } from './generated/core/db/sender_sweep/SweepSummary';
import type { ThreadSweepSender as WireThreadSweepSender } from './generated/core/db/sender_sweep/ThreadSweepSender';
import type { ListCleanupSendersInput } from './generated/app/commands/sender_sweep/ListCleanupSendersInput';
import type { SenderSweepTargetsInput } from './generated/app/commands/sender_sweep/SenderSweepTargetsInput';
import type { UpsertSweepRuleInput } from './generated/app/commands/sender_sweep/UpsertSweepRuleInput';
import { invoke } from './core';

/** A sender in the cleanup view: recent Inbox volume and what can be done about it. */
export type BackendCleanupSender = WireCleanupSender;
/** One Inbox thread a sender sweep takes, with the state it leaves. */
export type BackendSweepTarget = WireSweepTarget;
/** A recurring "from now on" rule for one sender in one account. */
export type BackendSweepRule = WireSweepRuleRecord;
export type BackendSweepAction = 'archive' | 'trash' | 'move';
/** How much a sweep takes and what it leaves alone. */
export type BackendSweepSummary = WireSweepSummary;
/** Who started a thread. */
export type BackendThreadSweepSender = WireThreadSweepSender;

export type BackendUpsertSweepRule = Omit<UpsertSweepRuleInput, 'action'> & { action: BackendSweepAction };

export async function backendListCleanupSenders(
  input: ListCleanupSendersInput = {},
): Promise<BackendCleanupSender[]> {
  return invoke<BackendCleanupSender[]>('list_cleanup_senders', { input });
}

/** The sender's Inbox threads a sweep would file, newest first. */
export async function backendListSenderSweepTargets(
  input: SenderSweepTargetsInput,
): Promise<BackendSweepTarget[]> {
  return invoke<BackendSweepTarget[]>('list_sender_sweep_targets', { input });
}

/** The counts of `backendListSenderSweepTargets`, without shipping the ids. */
export async function backendSenderSweepSummary(input: SenderSweepTargetsInput): Promise<BackendSweepSummary> {
  return invoke<BackendSweepSummary>('sender_sweep_summary', { input });
}

/** Who started the thread: its earliest message that is not the user's own. */
export async function backendThreadSweepSender(threadId: string): Promise<BackendThreadSweepSender | null> {
  return invoke<BackendThreadSweepSender | null>('thread_sweep_sender', { threadId });
}

export async function backendListSweepRules(): Promise<BackendSweepRule[]> {
  return invoke<BackendSweepRule[]>('list_sweep_rules');
}

/** Saves the rule (one per sender and account) and runs it once right away. */
export async function backendUpsertSweepRule(input: BackendUpsertSweepRule): Promise<BackendSweepRule> {
  return invoke<BackendSweepRule>('upsert_sweep_rule', { input });
}

export async function backendDeleteSweepRule(ruleId: string): Promise<void> {
  return invoke<void>('delete_sweep_rule', { ruleId });
}
