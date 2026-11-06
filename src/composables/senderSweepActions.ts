/**
 * The one-shot sender sweep behind `useThreadActions().sweepSender`, loaded on
 * first use so the startup bundle does not carry it.
 *
 * The backend names the threads — most are not loaded — and they go through
 * the same Done and Trash as a selection, in batches, with one receipt and one
 * Undo for the whole sweep.
 */
import {
  backendListSenderSweepTargets,
  backendUndoTrash,
  type BackendSweepTarget,
  type BackendTrashReceipt,
} from '@/services/backend';
import type { useThreadsStore } from '@/stores/threads';
import type { useToast } from '@/composables/useToast';
import type { useWorkflow } from '@/composables/useWorkflow';
import type { createThreadActionQueue } from './threadActionQueue';
import type { StateSnapshot } from './useThreadActions';
import type { WorkflowState } from '@/types/workflow';
import { t, tc } from '@/i18n/runtime';

/**
 * Threads per request, the backend's `SWEEP_BATCH`. A sweep of thousands of
 * threads is a few dozen bounded requests the list stays responsive between,
 * not one request that holds the database for seconds.
 */
export const SENDER_SWEEP_BATCH = 200;

export type SenderSweepAction = 'archive' | 'trash';

export interface SenderSweepInput {
  accountId: string;
  email: string;
  action: SenderSweepAction;
  keepLatest?: boolean;
}

/** What `useThreadActions` lends the sweep: its queue, receipts and refresh. */
export interface SenderSweepContext {
  queue: ReturnType<typeof createThreadActionQueue>;
  threads: ReturnType<typeof useThreadsStore>;
  workflow: ReturnType<typeof useWorkflow>;
  toast: ReturnType<typeof useToast>;
  undoTimeoutMs: number;
  showWorkflowUndo: (
    snapshot: StateSnapshot[],
    nextState: WorkflowState,
    label: string,
    options?: { reveal?: boolean; message?: (count: number) => string },
  ) => void;
  workflowResultMessage: (count: number, state: WorkflowState, fallback: string) => string;
  continueTriageAfterFiling: (threadIds: string[]) => void;
  refreshLists: () => Promise<unknown>;
}

function batches<T>(items: readonly T[], size = SENDER_SWEEP_BATCH): T[][] {
  const out: T[][] = [];
  for (let start = 0; start < items.length; start += size) out.push(items.slice(start, start + size));
  return out;
}

export async function runSenderSweep(ctx: SenderSweepContext, input: SenderSweepInput): Promise<number> {
  let targets: BackendSweepTarget[];
  try {
    targets = await backendListSenderSweepTargets({
      account_id: input.accountId,
      sender_email: input.email,
      keep_latest: input.keepLatest ?? false,
      older_than_days: null,
    });
  } catch (error) {
    ctx.toast.error(error instanceof Error ? error.message : t('sweep.result.failed'));
    return 0;
  }
  if (targets.length === 0) {
    ctx.toast.info(t('sweep.result.nothing', { sender: input.email }));
    return 0;
  }
  return input.action === 'archive' ? archiveSweep(ctx, targets) : trashSweep(ctx, targets);
}

function archiveSweep(ctx: SenderSweepContext, targets: BackendSweepTarget[]): Promise<number> {
  const ids = targets.map((target) => target.thread_id);
  const knownFrom = Object.fromEntries(targets.map((target) => [target.thread_id, target.workflow_state]));
  return ctx.queue.run(ids, async () => {
    const applied: string[] = [];
    for (const batch of batches(ids)) {
      applied.push(...await ctx.workflow.transitionMany(batch, 'done', 'manual', undefined, {
        refresh: false,
        knownFrom,
      }));
    }
    const appliedIds = new Set(applied);
    const snapshot: StateSnapshot[] = targets
      .filter((target) => appliedIds.has(target.thread_id))
      .map((target) => ({
        threadId: target.thread_id,
        previousState: target.workflow_state,
        ...(target.unread_count > 0 ? { wasUnread: true } : {}),
      }));
    if (snapshot.length === 0) return 0;
    const total = targets.length;
    const label = t('thread.state.done');
    // A receipt that says 180 when 200 were asked for has to say so.
    ctx.showWorkflowUndo(snapshot, 'done', label, {
      reveal: false,
      message: (count) => count === total
        ? ctx.workflowResultMessage(count, 'done', label)
        : t('sweep.result.archivedOf', { count, total }),
    });
    ctx.threads.deselectThreads(ids);
    ctx.continueTriageAfterFiling(ids);
    await ctx.refreshLists();
    return snapshot.length;
  });
}

function trashSweep(ctx: SenderSweepContext, targets: BackendSweepTarget[]): Promise<number> {
  const ids = targets.map((target) => target.thread_id);
  return ctx.queue.run(ids, async () => {
    ids.forEach((threadId) => ctx.threads.focusNeighborThread(threadId));
    const receipts: BackendTrashReceipt[] = [];
    for (const batch of batches(ids)) {
      try {
        receipts.push(await ctx.threads.trashThreads(batch));
      } catch (error) {
        // The batches before this one stand; their Undo follows below.
        ctx.toast.error(error instanceof Error ? error.message : t('thread.result.trashFailed'));
        break;
      }
    }
    ctx.threads.deselectThreads(ids);
    ctx.continueTriageAfterFiling(ids);
    const undoIds = receipts.flatMap((receipt) => receipt.undo_thread_ids);
    const tokens = receipts.flatMap((receipt) => receipt.undo_token ? [receipt.undo_token] : []);
    const deleted = receipts.reduce((sum, receipt) => sum + receipt.permanently_deleted.length, 0);
    if (deleted > 0) ctx.toast.info(tc('thread.result.permanentlyDeleted', deleted));
    if (undoIds.length === 0) return deleted;
    const message = undoIds.length === ids.length
      ? tc('sweep.result.trashed', undoIds.length)
      : t('sweep.result.trashedOf', { count: undoIds.length, total: ids.length });
    if (tokens.length === 0) {
      ctx.toast.info(message, { timeout: ctx.undoTimeoutMs });
      return undoIds.length;
    }
    ctx.toast.success(message, {
      timeout: ctx.undoTimeoutMs,
      action: {
        label: t('thread.actions.undo'),
        present: 'fab',
        handler: () => ctx.queue.run(undoIds, async () => {
          try {
            for (const token of tokens) await backendUndoTrash(token);
            await ctx.refreshLists();
            return true;
          } catch (err) {
            ctx.toast.error(err instanceof Error ? err.message : t('thread.result.restoreFailed'));
            await ctx.refreshLists();
            return false;
          }
        }),
      },
    });
    return undoIds.length + deleted;
  });
}
