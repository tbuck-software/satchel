import type { WorkflowState } from '@/types/workflow';
import type { BatchTransitionResult } from '@/services/backend';
import type { WorkflowRowMutation } from '@/stores/threads/collection';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useThreadsStore } from '@/stores/threads';
import { useWorkflowStore } from '@/stores/workflow';
import { useToast } from '@/composables/useToast';
import {
  backendDismissMobileNotifications,
  backendSetThreadsRead,
  backendTransitionThreads,
} from '@/services/backend';
import { recordFrontendDiagnostic } from '@/utils/diagnostics';
import { t, tc } from '@/i18n/runtime';

/**
 * How long a checked-off row animates out before it is removed from the list.
 * Must match the `thread-row-leave` animation in ThreadListItem.vue.
 */
export const THREAD_LEAVE_MS = 280;
export const WORKFLOW_PERSISTENCE_CONCURRENCY = 8;

let activePersistenceTasks = 0;
const persistenceWaiters: Array<() => void> = [];

async function withPersistenceSlot<T>(task: () => Promise<T>): Promise<T> {
  if (activePersistenceTasks >= WORKFLOW_PERSISTENCE_CONCURRENCY) {
    await new Promise<void>((resolve) => persistenceWaiters.push(resolve));
  } else {
    activePersistenceTasks += 1;
  }

  try {
    return await task();
  } finally {
    const next = persistenceWaiters.shift();
    if (next) next();
    else activePersistenceTasks -= 1;
  }
}

/** Keys, not text: a translated table here would freeze the language. */
const STATE_LABEL_KEYS: Record<WorkflowState, string> = {
  inbox: 'thread.state.inbox',
  todo: 'thread.state.todo',
  waiting: 'thread.state.waiting',
  aside: 'thread.state.aside',
  done: 'thread.state.done',
  snoozed: 'thread.state.snoozed',
};

function stateLabel(state: WorkflowState): string {
  return t(STATE_LABEL_KEYS[state]);
}

function workflowFailureMessage(count: number, state: WorkflowState, detail: string): string {
  const label = stateLabel(state);
  if (count === 1) return t('thread.workflow.failedOne', { label, detail });
  return t('thread.workflow.failedMany', { count, label, detail });
}

const WORKFLOW_STATES: readonly WorkflowState[] = [
  'inbox',
  'todo',
  'waiting',
  'aside',
  'done',
  'snoozed',
];

/**
 * The transition compare-and-swap names the authoritative state whenever a
 * request loses a race (a rule, a blocked sender, another device, or the
 * folder reconcile resurfacing already-done mail). Message format asserted
 * by the Rust workflow suite.
 */
function concurrentTransitionFoundState(message: string): WorkflowState | null {
  const match =
    /workflow state changed concurrently \(expected \w+, found (\w+)\)/.exec(message);
  const found = match?.[1];
  return found && (WORKFLOW_STATES as string[]).includes(found)
    ? (found as WorkflowState)
    : null;
}

function readFailureMessage(count: number, state: WorkflowState, detail: string): string {
  return tc('thread.workflow.readFailed', count, {
    state: stateLabel(state).toLowerCase(),
    detail,
  });
}

/** Destinations that file the row away, so it animates out of the list. */
function transitionAnimatesOut(from: WorkflowState, to: WorkflowState): boolean {
  return to === 'done'
    || to === 'snoozed'
    || to === 'waiting'
    || to === 'aside'
    || (from === 'aside' && to === 'inbox');
}

function leaveAnimationDelay(): number {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return THREAD_LEAVE_MS;
  }
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : THREAD_LEAVE_MS;
}

interface TransitionTarget {
  threadId: string;
  from: WorkflowState;
  previousUnreadCount: number;
}

function rollbacks(targets: TransitionTarget[]): WorkflowRowMutation[] {
  return targets.map((target) => ({
    threadId: target.threadId,
    state: target.from,
    unreadCount: target.previousUnreadCount,
  }));
}

export function useWorkflow() {
  const threadsStore = useThreadsStore();
  const mailboxesStore = useMailboxesStore();
  const workflowStore = useWorkflowStore();
  const toast = useToast();

  /**
   * Move a whole selection in one round trip.
   *
   * The per-thread path costs three IPC calls and three reactive publications
   * *per thread*; filing a date section that way took hundreds of round trips
   * and re-ran the list and contact projections between each of them.
   * Resolves to the thread ids the backend actually changed.
   */
  async function transitionMany(
    threadIds: string[],
    to: WorkflowState,
    reason: 'manual' | 'rule' | 'snooze_expired' | 'reply_received' = 'manual',
    snoozeUntil?: string | null,
    options: {
      refresh?: boolean;
      /** Undo supplies the confirmed state from its receipt, independent of loaded rows. */
      restoreFrom?: Record<string, WorkflowState>;
    } = {},
  ): Promise<string[]> {
    const rowsById = new Map(threadsStore.threads.map((thread) => [thread.id, thread]));
    const leaving = new Set(threadsStore.leavingThreadIds);
    const targets: TransitionTarget[] = [];
    const seen = new Set<string>();
    for (const threadId of threadIds) {
      if (seen.has(threadId)) continue;
      seen.add(threadId);
      const thread = rowsById.get(threadId);
      // Still animating out from a previous check-off — a second trigger
      // during that short interval would file it twice.
      const from = options.restoreFrom?.[threadId] ?? thread?.workflowState;
      if (!from || (!options.restoreFrom && (from === to || leaving.has(threadId)))) continue;
      targets.push({
        threadId,
        from,
        previousUnreadCount: thread?.unreadCount ?? 0,
      });
    }
    if (targets.length === 0) return [];

    const animating = targets.filter((target) => rowsById.has(target.threadId) && !options.restoreFrom
      && transitionAnimatesOut(target.from, to));
    if (animating.length > 0) {
      for (const target of animating) {
        threadsStore.focusNeighborThread(target.threadId);
        threadsStore.beginThreadLeave(target.threadId, to);
      }
      // One animation for the batch, not one per row.
      await new Promise((resolve) => setTimeout(resolve, leaveAnimationDelay()));
    }

    threadsStore.applyWorkflowStates(targets.map((target) => ({
      threadId: target.threadId,
      state: to,
      snoozeUntil,
      unreadCount: to === 'done' ? 0 : undefined,
    })));
    for (const target of animating) threadsStore.endThreadLeave(target.threadId);

    const track = (applied: TransitionTarget[]) => {
      const at = new Date().toISOString();
      for (const target of applied) {
        workflowStore.trackTransition({ from: target.from, to, reason, at });
      }
    };

    let outcome: BatchTransitionResult;
    try {
      outcome = await withPersistenceSlot(() => backendTransitionThreads({
        threads: targets.map((target) => ({ thread_id: target.threadId, from: target.from })),
        to,
        snooze_until: snoozeUntil ?? null,
      }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      recordFrontendDiagnostic(
        'workflow.transition_failed',
        `threads=${targets.length} to=${to} error=${message}`,
      );
      threadsStore.applyWorkflowStates(rollbacks(targets));
      toast.error(workflowFailureMessage(targets.length, to, message), {
        groupKey: `workflow-error:${to}:${message}`,
        count: targets.length,
        groupMessage: (count) => workflowFailureMessage(count, to, message),
      });
      return [];
    }

    const targetsById = new Map(targets.map((target) => [target.threadId, target]));
    const applied = outcome.applied.flatMap((threadId) => {
      const target = targetsById.get(threadId);
      return target ? [target] : [];
    });
    const reverted: TransitionTarget[] = [];
    const adopted: WorkflowRowMutation[] = [];
    const discarded: string[] = [];
    const failuresByMessage = new Map<string, number>();
    for (const failure of outcome.failed) {
      const target = targetsById.get(failure.thread_id);
      if (!target) continue;
      // Sync may merge or delete a thread between a list refresh and the
      // click. Keeping that confirmed-missing projection would leave a
      // permanent ghost row whose every action fails.
      if (failure.error === 'thread not found') {
        discarded.push(target.threadId);
        continue;
      }
      const found = concurrentTransitionFoundState(failure.error);
      if (found === to) {
        // The requested end state already holds on the server — a stale list
        // projection re-offered an already-filed thread. Converge quietly.
        recordFrontendDiagnostic('workflow.transition_converged', `thread=${target.threadId} to=${to}`);
        applied.push(target);
        continue;
      }
      if (found) {
        // A genuine divergence (a rule, another device, …). Adopt the
        // authoritative state instead of this client's outdated belief.
        recordFrontendDiagnostic('workflow.conflict_adopted', `thread=${target.threadId} found=${found}`);
        adopted.push({
          threadId: target.threadId,
          state: found,
          unreadCount: target.previousUnreadCount,
        });
        continue;
      }
      recordFrontendDiagnostic(
        'workflow.transition_failed',
        `thread=${target.threadId} from=${target.from} to=${to} error=${failure.error}`,
      );
      reverted.push(target);
      failuresByMessage.set(failure.error, (failuresByMessage.get(failure.error) ?? 0) + 1);
    }

    // One publication for every correction the backend asked for.
    threadsStore.applyWorkflowStates([...rollbacks(reverted), ...adopted]);
    for (const threadId of discarded) threadsStore.discardThread(threadId);
    for (const [message, count] of failuresByMessage) {
      toast.error(workflowFailureMessage(count, to, message), {
        groupKey: `workflow-error:${to}:${message}`,
        count,
        groupMessage: (grouped) => workflowFailureMessage(grouped, to, message),
      });
    }

    const appliedIds = applied.map((target) => target.threadId);
    if (appliedIds.length > 0 && to === 'done') {
      try {
        await withPersistenceSlot(async () => {
          await backendSetThreadsRead(appliedIds, true);
          await backendDismissMobileNotifications(appliedIds).catch(() => {});
        });
      } catch (err) {
        // The workflow transition is durable and its provider move is queued.
        // A later read failure must not pretend the whole action failed.
        const message = err instanceof Error ? err.message : String(err);
        toast.warning(readFailureMessage(appliedIds.length, to, message), {
          groupKey: `workflow-read-error:${to}:${message}`,
          count: appliedIds.length,
          groupMessage: (count) => readFailureMessage(count, to, message),
        });
      }
    }

    if (options.refresh !== false && (appliedIds.length > 0 || adopted.length > 0)) {
      // Merge refresh, not load(): the optimistic flip has already updated the
      // list, and a full reload replaces the array with page one.
      await Promise.all([threadsStore.refreshHead(), mailboxesStore.load()]);
    }
    track(applied);
    return appliedIds;
  }

  return {
    transitionMany,
  };
}
