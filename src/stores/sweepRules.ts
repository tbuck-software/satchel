import { defineStore } from 'pinia';
import { ref } from 'vue';
import { orderedLoad } from './internal/orderedLoad';
import {
  backendCommandsAvailable,
  backendDeleteSweepRule,
  backendListCleanupSenders,
  backendListSweepRules,
  backendSenderSweepSummary,
  backendThreadSweepSender,
  backendUpsertSweepRule,
  type BackendCleanupSender,
  type BackendSweepRule,
  type BackendSweepSummary,
  type BackendThreadSweepSender,
  type BackendUpsertSweepRule,
} from '@/services/backend';

export type CleanupSender = BackendCleanupSender;
export type SweepRule = BackendSweepRule;
export type SweepSummary = BackendSweepSummary;

/**
 * Sender sweeps. The recurring rules ("from now on: archive mail from this
 * sender older than 10 days") are stored and run by the backend; this mirrors
 * their list for Settings and the sender rows, and carries the two reads the
 * cleanup view and the sweep dialog need.
 */
export const useSweepRulesStore = defineStore('sweepRules', () => {
  const items = ref<BackendSweepRule[]>([]);
  const loader = orderedLoad();
  const { loading, loadFailed } = loader;

  async function load() {
    if (!backendCommandsAvailable()) return;
    await loader.load(backendListSweepRules, (rows) => { items.value = rows; });
  }

  function forSender(accountId: string, email: string): BackendSweepRule | undefined {
    const normalized = email.trim().toLowerCase();
    return items.value.find((rule) => rule.account_id === accountId && rule.sender_email === normalized);
  }

  /** One rule per sender and account: saving again replaces the earlier one. */
  async function save(input: BackendUpsertSweepRule): Promise<BackendSweepRule> {
    return loader.mutate(async () => {
      const rule = await backendUpsertSweepRule(input);
      const index = items.value.findIndex((item) => item.id === rule.id);
      if (index >= 0) items.value.splice(index, 1, rule);
      else items.value.push(rule);
      return rule;
    });
  }

  async function remove(id: string): Promise<void> {
    await loader.mutate(async () => {
      await backendDeleteSweepRule(id);
      items.value = items.value.filter((item) => item.id !== id);
    });
  }

  /** The cleanup view's senders: Inbox volume over the last `days`, busiest first. */
  async function listSenders(input: { days: number; limit: number; accountId?: string | null }): Promise<BackendCleanupSender[]> {
    if (!backendCommandsAvailable()) return [];
    return backendListCleanupSenders({ days: input.days, limit: input.limit, account_id: input.accountId || null });
  }

  /** How much a sweep (one-shot, or a rule run now) would file, and what it skips. */
  function previewSummary(input: {
    accountId: string;
    email: string;
    keepLatest?: boolean;
    olderThanDays?: number | null;
  }): Promise<BackendSweepSummary> {
    return backendSenderSweepSummary({
      account_id: input.accountId,
      sender_email: input.email,
      keep_latest: input.keepLatest ?? false,
      older_than_days: input.olderThanDays ?? null,
    });
  }

  /** Who started a thread; `null` when only the user wrote in it. */
  async function threadSender(threadId: string): Promise<BackendThreadSweepSender | null> {
    if (!backendCommandsAvailable()) return null;
    return backendThreadSweepSender(threadId);
  }

  return { items, loading, loadFailed, load, forSender, save, remove, listSenders, previewSummary, threadSender };
});
