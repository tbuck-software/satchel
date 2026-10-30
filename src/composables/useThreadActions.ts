import { nextTick } from 'vue';
import { focusListThread } from './useListKeyboardNavigation';
import { useRoute, useRouter } from 'vue-router';
import type { WorkflowState } from '@/types/workflow';
import {
  backendBlockSender,
  backendMoveThreadsToFolder,
  backendUndoTrash,
  type BackendTrashReceipt,
} from '@/services/backend';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useThreadsStore } from '@/stores/threads';
import { useTopicsStore } from '@/stores/topics';
import { useToast } from '@/composables/useToast';
import { useWorkflow } from '@/composables/useWorkflow';
import { createThreadActionQueue } from './threadActionQueue';
import { t, tc } from '@/i18n/runtime';
import { openView } from '@/router/mailViews';

interface StateSnapshot {
  threadId: string;
  previousState: WorkflowState;
  snoozeUntil?: string;
  /** State confirmed by the action; used for undo even after row eviction. */
  expectedState?: WorkflowState;
}

const actionQueues = new WeakMap<ReturnType<typeof useThreadsStore>, ReturnType<typeof createThreadActionQueue>>();

const UNDO_TIMEOUT_MS = 9000;

function workflowResultMessage(count: number, state: WorkflowState, fallback: string): string {
  switch (state) {
    case 'done':
      return tc('thread.result.done', count);
    case 'snoozed':
      return tc('thread.result.snoozed', count);
    case 'aside':
      return tc('thread.result.aside', count);
    case 'waiting':
      return tc('thread.result.waiting', count);
    case 'inbox':
      return tc('thread.result.inbox', count);
    default:
      return tc('thread.result.fallback', count, { label: fallback });
  }
}

/**
 * Thread actions with optimistic updates and a consistent undo affordance.
 *
 * This exists because archive/trash used to behave differently depending on how
 * they were triggered: the mouse path in InboxView captured a snapshot and
 * offered undo, while the keyboard path called the backend directly and then
 * reloaded the whole list — unrecoverable, and the slowest possible refresh on
 * the very flow that is meant to be the fast one. Both paths now go through
 * here.
 */
export function useThreadActions() {
  const route = useRoute();
  const router = useRouter();
  const threads = useThreadsStore();
  const mailboxes = useMailboxesStore();
  const topics = useTopicsStore();
  const workflow = useWorkflow();
  const toast = useToast();

  const queue = actionQueues.get(threads) ?? createThreadActionQueue();
  actionQueues.set(threads, queue);

  function snapshotStates(threadIds: string[]): StateSnapshot[] {
    return threadIds.flatMap((threadId) => {
      const thread = threads.find(threadId);
      return thread ? [{ threadId, previousState: thread.workflowState, snoozeUntil: thread.snoozeUntil }] : [];
    });
  }

  async function restorePlace(threadIds: string[]) {
    const ids = [...new Set(threadIds)];
    const first = ids[0];
    if (!first) return;
    for (const id of ids) {
      if (!threads.find(id)) await threads.ensureThreadLoaded(id);
    }
    // Flush projection watchers before restoring selection. Revealing a bucket
    // can clear selection too, so the list gets its turn before we reselect.
    await nextTick();
    await focusListThread(first);
    threads.selectThread(first);
    threads.selectedThreadIds = ids.length > 1 ? ids : [];
    if (route?.name === 'thread') {
      await router?.replace({ name: 'thread', params: { id: first } });
    }
  }

  async function restoreStatesNow(snapshot: StateSnapshot[]) {
    if (snapshot.length === 0) return false;
    // Batch only threads whose previous state and wake time both match.
    const groups = new Map<string, { snapshot: StateSnapshot; threadIds: string[]; restoreFrom: Record<string, WorkflowState> }>();
    for (const entry of snapshot) {
      const key = JSON.stringify([entry.previousState, entry.snoozeUntil]);
      const group = groups.get(key);
      if (group) {
        group.threadIds.push(entry.threadId);
        group.restoreFrom[entry.threadId] = entry.expectedState ?? entry.previousState;
      } else groups.set(key, {
        snapshot: entry,
        threadIds: [entry.threadId],
        restoreFrom: { [entry.threadId]: entry.expectedState ?? entry.previousState },
      });
    }
    const restored = await Promise.all(
      [...groups.values()].map(({ snapshot: entry, threadIds, restoreFrom }) =>
        workflow.transitionMany(threadIds, entry.previousState, 'manual', entry.snoozeUntil, {
          refresh: false,
          restoreFrom,
        }),
      ),
    );
    if (restored.flat().length === 0) {
      toast.error(t('thread.result.restoreEmpty'));
      return false;
    }
    await refreshLists();
    const restoredIds = new Set(restored.flat());
    await restorePlace(snapshot.filter(entry => restoredIds.has(entry.threadId)).map(entry => entry.threadId));
    return true;
  }

  function restoreStates(snapshot: StateSnapshot[]) {
    return queue.run(snapshot.map((entry) => entry.threadId), () => restoreStatesNow(snapshot));
  }

  function showWorkflowUndo(
    snapshot: StateSnapshot[],
    nextState: WorkflowState,
    label: string,
  ) {
    snapshot = snapshot.map((entry) => ({ ...entry, expectedState: nextState }));
    toast.success(workflowResultMessage(snapshot.length, nextState, label), {
      timeout: UNDO_TIMEOUT_MS,
      groupKey: `workflow:${nextState}`,
      count: snapshot.length,
      groupMessage: (count) => workflowResultMessage(count, nextState, label),
      action: {
        label: t('thread.actions.undo'),
        present: 'fab',
        handler: () => restoreStates(snapshot),
        batch: {
          key: 'workflow:restore',
          items: snapshot,
          handler: (items) => restoreStates(items as StateSnapshot[]),
        },
      },
    });
  }

  /**
   * One refresh for the whole batch. Every thread used to pull the full list
   * on its own, so archiving a smart folder of 20 mails ran 20 list queries —
   * the expensive one — back to back.
   *
   * refreshHead, not load: the optimistic updates have already changed the
   * list, and load() replaces the array with page one — dropping paged-in
   * tails and yanking the scroll right after every action.
   */
  async function refreshLists() {
    await Promise.all([threads.refreshHead(), mailboxes.load()]);
  }

  /** Move threads to a workflow state with undo. */
  /**
   * Single pane: a thread you just filed is no longer a place to stand.
   *
   * With three columns the reader simply shows the next thread and the list
   * stays put beside it. On a phone the thread *is* the screen, so filing it
   * leaves the user looking at something they have already dealt with, with no
   * indication that anything happened beyond a toast.
   */
  function continueTriageAfterFiling(threadIds: string[]) {
    if (route?.name !== 'thread') return;
    const open = route.params.id;
    if (typeof open !== 'string' || !threadIds.includes(open)) return;

    const nextThreadId = threads.activeThreadId;
    if (nextThreadId && !threadIds.includes(nextThreadId)) {
      void router?.replace({ name: 'thread', params: { id: nextThreadId } });
      return;
    }
    void router?.push({ name: 'inbox' });
  }

  /**
   * Resolves `true` once every thread is in `nextState`, whether this call
   * moved it or it already was there; `false` when any was refused, rolled
   * back or not found.
   */
  async function transitionThreads(
    threadIds: string[],
    nextState: WorkflowState,
    label: string,
    snoozeUntil?: string,
  ): Promise<boolean> {
    if (threadIds.length === 0) return true;

    const normalizedThreadIds = Array.from(new Set(threadIds));
    const knownIds = snapshotStates(normalizedThreadIds).map((entry) => entry.threadId);
    return queue.run(normalizedThreadIds, async () => {
      // A preceding move may evict this row while this action waits in its lane.
      for (const id of knownIds) {
        if (!threads.find(id)) await threads.ensureThreadLoaded(id);
      }
      const requested = snapshotStates(normalizedThreadIds);
      if (requested.length === 0) return false;
      const applied = await workflow.transitionMany(
        normalizedThreadIds, nextState, 'manual', snoozeUntil, { refresh: false },
      );

      // Offering undo for a thread the backend refused would be a lie — and the
      // failure already announced itself with its own toast.
      const appliedIds = new Set(applied);
      const snapshot = requested.filter((entry) => appliedIds.has(entry.threadId));
      const complete = requested.length === normalizedThreadIds.length && requested.every(
        (entry) => appliedIds.has(entry.threadId) || entry.previousState === nextState,
      );
      if (snapshot.length === 0) return complete;

      // One receipt with one deadline for the whole batch, and one Undo payload.
      showWorkflowUndo(snapshot, nextState, label);
      if (nextState === 'waiting') showWhereWaitingWent();

      // A smart-folder close can publish an older Inbox page during persistence.
      // Reassert confirmed states before refreshing that projection.
      threads.applyWorkflowStates(snapshot.map((entry) => ({
        threadId: entry.threadId,
        state: nextState,
        snoozeUntil,
        unreadCount: nextState === 'done' ? 0 : undefined,
      })));

      // The user may have selected other threads while this one was in flight.
      threads.deselectThreads(normalizedThreadIds);
      continueTriageAfterFiling(threadIds);

      // Each successful transition already started its own visible Undo period.
      // A slow list refresh must not consume the user's chance to cancel it.
      await refreshLists();
      return complete;
    });
  }

  /** Waiting mail leaves the list; say where it went, unless that is on screen. */
  function showWhereWaitingWent() {
    if (threads.workflowFilter === 'later') return;
    toast.info(t('thread.result.inLater'), {
      groupKey: 'later:show',
      action: { label: t('common.show'), handler: () => openView(router, 'later') },
    });
  }

  /**
   * Archive = the "done" workflow state; fully reversible. Resolves `false`
   * when a thread did not reach it; the failure has its own toast.
   */
  async function archive(threadIds: string[]): Promise<boolean> {
    return transitionThreads(threadIds, 'done', t('thread.state.done'));
  }

  async function snooze(threadIds: string[]): Promise<void> {
    await transitionThreads(threadIds, 'snoozed', t('thread.state.snoozed'));
  }

  async function snoozeAt(threadIds: string[], snoozeUntil: string): Promise<void> {
    await transitionThreads(threadIds, 'snoozed', t('thread.state.snoozed'), snoozeUntil);
  }

  async function waitForReply(threadIds: string[]): Promise<void> {
    await transitionThreads(threadIds, 'waiting', t('thread.state.waiting'));
  }

  async function setAside(threadIds: string[]): Promise<void> {
    await transitionThreads(threadIds, 'aside', t('thread.state.aside'));
  }

  async function moveToInbox(threadIds: string[]): Promise<void> {
    await transitionThreads(threadIds, 'inbox', t('thread.state.inbox'));
  }

  /** Block a sender; the backend atomically files matching threads remotely and locally. */
  async function blockSender(input: {
    pattern: string;
    accountId?: string | null;
    threadIds?: string[];
  }): Promise<void> {
    await backendBlockSender({ pattern: input.pattern, accountId: input.accountId });

    const threadIds = Array.from(
      new Set((input.threadIds ?? []).map((id) => id.trim()).filter(Boolean)),
    );
    if (threadIds.length === 0) return;

    // Blocking is remote-first: do not move the local focus until the backend
    // has durably queued the provider move and committed its local projection.
    threadIds.forEach((threadId) => threads.focusNeighborThread(threadId));
    await refreshLists();
  }

  /** Move a batch to a provider folder, then reload the changed projection. */
  async function moveToFolder(
    threadIds: string[],
    targetFolderId: string,
    targetLabel?: string,
  ): Promise<void> {
    const normalizedThreadIds = Array.from(new Set(threadIds.map((id) => id.trim()).filter(Boolean)));
    if (normalizedThreadIds.length === 0 || !targetFolderId) return;

    return queue.run(normalizedThreadIds, async () => {
      try {
        // The command resolves and queues provider references before changing
        // its local rows. Keep the frontend projection untouched until that
        // remote-first operation succeeds as well.
        await backendMoveThreadsToFolder({
          threadIds: normalizedThreadIds,
          targetFolderId,
        });
        await Promise.all([threads.load(), mailboxes.load()]);
        toast.success(t('thread.move.moved', {
          folder: targetLabel || t('thread.move.fallbackFolder'),
        }));
      } catch (error) {
        toast.error(t('thread.move.failed', {
          error: error instanceof Error ? error.message : String(error),
        }));
      }
    });
  }

  /**
   * Trash and spam share one shape: the store removes the rows optimistically,
   * the backend returns a receipt, and Undo hands that receipt back. The
   * receipt owns exact source folders and provider delivery state.
   */
  async function fileWithReceipt(
    threadIds: string[],
    file: (threadIds: string[]) => Promise<BackendTrashReceipt>,
    messages: { failed: string; result: (count: number) => string },
  ): Promise<void> {
    if (threadIds.length === 0) return;

    const knownIds = snapshotStates(threadIds).map((entry) => entry.threadId);
    return queue.run(threadIds, async () => {
      for (const id of knownIds) {
        if (!threads.find(id)) await threads.ensureThreadLoaded(id);
      }
      const snapshot = snapshotStates(threadIds);
      if (snapshot.length === 0) return;

      threadIds.forEach((threadId) => threads.focusNeighborThread(threadId));

      let receipt: BackendTrashReceipt;
      try {
        // The store drops the threads from the list first and puts them back if
        // the backend refuses, so they are gone from the view immediately
        // instead of after a full reload of a large mailbox.
        receipt = await file(threadIds);
      } catch (err) {
        toast.error(err instanceof Error ? err.message : messages.failed);
        return;
      }

      threads.deselectThreads(threadIds);
      continueTriageAfterFiling(threadIds);

      if (receipt.permanently_deleted.length > 0) {
        toast.info(tc('thread.result.permanentlyDeleted', receipt.permanently_deleted.length));
      }
      if (receipt.undo_thread_ids.length === 0) return;
      const message = messages.result(receipt.undo_thread_ids.length);
      const undoToken = receipt.undo_token;
      if (!undoToken) {
        toast.info(message, { timeout: UNDO_TIMEOUT_MS });
        return;
      }

      toast.success(message, {
        timeout: UNDO_TIMEOUT_MS,
        action: {
          label: t('thread.actions.undo'),
          present: 'fab',
          handler: () => queue.run(receipt.undo_thread_ids, async () => {
            try {
              await backendUndoTrash(undoToken);
              await refreshLists();
              await restorePlace(receipt.undo_thread_ids);
              return true;
            } catch (err) {
              toast.error(err instanceof Error ? err.message : t('thread.result.restoreFailed'));
              return false;
            }
          }),
        },
      });
    });
  }

  async function trash(threadIds: string[]): Promise<void> {
    await fileWithReceipt(threadIds, (ids) => threads.trashThreads(ids), {
      failed: t('thread.result.trashFailed'),
      result: (count) => tc('thread.result.trashed', count),
    });
  }

  /** Spam moves like trash, so it answers like trash: next thread, toast, Undo. */
  async function markSpam(threadIds: string[]): Promise<void> {
    await fileWithReceipt(threadIds, (ids) => threads.markThreadsSpam(ids), {
      failed: t('thread.result.spamFailed'),
      result: (count) => tc('thread.result.spammed', count),
    });
  }

  /** Memberships decide what an Undo may revert; read unknown ones first. */
  async function ensureTopicMemberships(threadIds: string[]) {
    const unknown = threadIds.filter((threadId) => !topics.hasMembershipsFor('thread', threadId));
    if (unknown.length === 0) return;
    try {
      await topics.loadMembershipsFor('thread', unknown);
    } catch {
      // Unknown memberships count as absent; the write itself still applies.
    }
  }

  /** Link threads to a topic; the store applies the optimistic membership. */
  async function addToTopic(threadIds: string[], topicId: string): Promise<void> {
    threadIds = Array.from(new Set(threadIds));
    if (threadIds.length === 0) return;
    const topicName = topics.byId(topicId)?.name ?? '';
    let added: string[];
    try {
      await ensureTopicMemberships(threadIds);
      added = await topics.addItemsToTopic(topicId, 'thread', threadIds);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : t('thread.result.addToTopicFailed'));
      return;
    }
    // Undo unlinks only what this action linked; earlier members stay.
    toast.success(
      tc('thread.result.addedToTopic', threadIds.length, { topic: topicName }),
      {
        timeout: UNDO_TIMEOUT_MS,
        ...(added.length > 0 ? {
          action: {
            label: t('thread.actions.undo'),
            handler: () => attempt(
              () => Promise.all(
                added.map((threadId) => topics.removeItemFromTopic(topicId, 'thread', threadId)),
              ),
              t('thread.result.removeFromTopicFailed'),
            ),
          },
        } : {}),
      },
    );
  }

  async function removeFromTopic(threadIds: string[], topicId: string): Promise<void> {
    threadIds = Array.from(new Set(threadIds));
    if (threadIds.length === 0) return;
    const topicName = topics.byId(topicId)?.name ?? '';
    let removed: string[];
    try {
      await ensureTopicMemberships(threadIds);
      const wasMember = await Promise.all(
        threadIds.map((threadId) => topics.removeItemFromTopic(topicId, 'thread', threadId)),
      );
      removed = threadIds.filter((_, index) => wasMember[index]);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : t('thread.result.removeFromTopicFailed'));
      return;
    }
    toast.success(t('thread.result.removedFromTopic', { topic: topicName }), {
      timeout: UNDO_TIMEOUT_MS,
      ...(removed.length > 0 ? {
        action: {
          label: t('thread.actions.undo'),
          present: 'fab',
          handler: () => attempt(
            () => topics.addItemsToTopic(topicId, 'thread', removed),
            t('thread.result.addToTopicFailed'),
          ),
        },
      } : {}),
    });
  }

  /**
   * Pinning maps to the provider's star/flag and remains independent of
   * priority. The optimistic write and its rollback belong to the store; this
   * only decides which rows change and reports a refusal.
   */
  async function setPinned(threadIds: string[], pinned: boolean): Promise<void> {
    const targets = threadIds.filter((threadId) =>
      threads.threads.some((entry) => entry.id === threadId && entry.isStarred !== pinned));
    if (targets.length === 0) return;

    const applied = await attempt(
      () => threads.setThreadsStarred(targets, pinned),
      t('thread.result.pinFailed'),
    );
    if (!applied) return;

    toast.success(t('thread.result.pinChanged'), {
      timeout: UNDO_TIMEOUT_MS,
      action: {
        label: t('thread.actions.undo'),
        present: 'fab',
        handler: () => attempt(async () => {
          await threads.setThreadsStarred(targets, !pinned);
          await threads.refreshHead();
          await restorePlace(threadIds);
        }),
      },
    });
    threads.deselectThreads(threadIds);
    await threads.refreshHead();
  }

  async function togglePinned(threadId: string): Promise<void> {
    const thread = threads.find(threadId);
    if (!thread) return;
    await setPinned([threadId], !thread.isStarred);
  }

  async function attempt(
    operation: () => Promise<unknown>,
    fallbackMessage?: string,
  ): Promise<boolean> {
    try {
      await operation();
      return true;
    } catch (error) {
      toast.error(error instanceof Error
        ? error.message
        : fallbackMessage ?? t('backend.errors.mailActionFailed'));
      return false;
    }
  }

  /** Toggles between normal (1) and high (2); the store rolls back a refusal. */
  async function togglePriority(threadId: string): Promise<void> {
    const thread = threads.find(threadId);
    if (!thread) return;
    await attempt(
      () => threads.setPriority(threadId, thread.priority >= 2 ? 1 : 2),
      t('thread.result.priorityFailed'),
    );
  }

  async function setRead(ids: string[], read: boolean, options: { undo?: boolean } = {}) {
    const changed = ids.filter(id => {
      const thread = threads.find(id);
      return thread && (thread.unreadCount > 0) === read;
    });
    const applied = await attempt(() => threads.setThreadsRead(ids, read));
    if (applied && options.undo && changed.length > 0) {
      toast.success(t('thread.result.readChanged'), {
        timeout: UNDO_TIMEOUT_MS,
        action: {
          label: t('thread.actions.undo'),
          present: 'fab',
          handler: () => attempt(async () => {
            await threads.setThreadsRead(changed, !read);
            await threads.refreshHead();
            await restorePlace(ids);
          }),
        },
      });
    }
    return applied;
  }
  async function setMuted(target: string | string[], muted: boolean) {
    const ids = typeof target === 'string' ? [target] : target;
    const changed = ids.filter(id => {
      const thread = threads.find(id);
      return thread && Boolean(thread.isMuted) !== muted;
    });
    const applied = await attempt(() => Promise.all(changed.map(id => threads.setThreadMuted(id, muted))));
    if (applied && changed.length > 0) {
      toast.success(t('thread.result.muteChanged'), {
        timeout: UNDO_TIMEOUT_MS,
        action: {
          label: t('thread.actions.undo'),
          present: 'fab',
          handler: () => attempt(async () => {
            await Promise.all(changed.map(id => threads.setThreadMuted(id, !muted)));
            await threads.refreshHead();
            await restorePlace(ids);
          }),
        },
      });
    }
    return applied;
  }

  return {
    attempt,
    setRead,
    setMuted,
    markSpam,
    archive,
    snooze,
    snoozeAt,
    waitForReply,
    setAside,
    moveToInbox,
    blockSender,
    moveToFolder,
    trash,
    setPinned,
    togglePinned,
    togglePriority,
    transitionThreads,
    addToTopic,
    removeFromTopic,
  };
}
