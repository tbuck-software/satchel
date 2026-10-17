import { computed, ref, watch, type ComputedRef, type Ref } from 'vue';
import type { Thread } from '@/types/mail';
import type { WorkflowState } from '@/types/workflow';
import { recordFrontendDiagnostic } from '@/utils/diagnostics';
import type { SelectionModifiers } from '@/utils/threadList';

/** What the threads store exposes of Selection. */
export interface ThreadSelectionSurface {
  activeThreadId: Ref<string>;
  activeThread: ComputedRef<Thread | undefined>;
  selectedThreadIds: Ref<string[]>;
  selectedThreads: ComputedRef<Thread[]>;
  leavingThreadIds: Ref<string[]>;
  leavingThreadStates: Ref<Partial<Record<string, WorkflowState>>>;
  selectThread(threadId: string): void;
  clearSelection(): void;
  setSelection(threadIds: string[]): void;
  selectAllVisibleThreads(): void;
  selectFromInteraction(
    threadId: string,
    modifiers: SelectionModifiers,
    openThread: boolean,
  ): void;
  toggleSelectionForThread(threadId: string): void;
  focusNeighborThread(threadId: string): void;
  nextThread(): void;
  prevThread(): void;
  beginThreadLeave(threadId: string, destination: WorkflowState): void;
  endThreadLeave(threadId: string): void;
}

export interface ThreadSelection {
  surface: ThreadSelectionSurface;
  reconcileRemovedThread(threadId: string): void;
  startWatching(): void;
}

/**
 * The visible order, as Filter+Buckets computes it.
 *
 * Selection is built first, so it cannot take Filter+Buckets as an argument.
 * Filter+Buckets installs its readers here instead when it is constructed.
 * Until then both read as empty, which is harmless: Selection only consults
 * them from handlers and from watchers the façade starts once everything
 * exists.
 */
export interface VisibleThreadProjection {
  scopedVisibleThreads(): Thread[];
  visibleThreadOrder(): string[];
}

export function createVisibleThreadProjection(): VisibleThreadProjection {
  return {
    scopedVisibleThreads: () => [],
    visibleThreadOrder: () => [],
  };
}

interface ThreadSelectionDependencies {
  threads: Ref<Thread[]>;
  find(threadId: string): Thread | undefined;
  projection: VisibleThreadProjection;
}

/**
 * Own active focus, bulk selection, keyboard navigation, and leave-animation
 * state for the thread collection.
 */
export function createThreadSelection({
  threads,
  find,
  projection,
}: ThreadSelectionDependencies): ThreadSelection {
  const activeThreadId = ref<string>(threads.value[0]?.id ?? '');
  const selectedThreadIds = ref<string[]>([]);
  const selectionAnchorId = ref<string>('');
  /**
   * Threads playing their leave animation. The row stays in the list while it
   * fades out; the workflow flip that actually removes it happens afterwards
   * (see useWorkflow). Also serves as a re-entry guard so a second check-off
   * during the animation does not archive twice.
   */
  const leavingThreadIds = ref<string[]>([]);
  /** Destination drives the right visual cue (check, snooze, set aside). */
  const leavingThreadStates = ref<Partial<Record<string, WorkflowState>>>({});

  const activeThread = computed(() => find(activeThreadId.value));
  const selectedThreadSet = computed(() => new Set(selectedThreadIds.value));
  const selectedThreads = computed(() =>
    threads.value.filter((thread) => selectedThreadSet.value.has(thread.id)),
  );

  function selectThread(threadId: string) {
    if (threadId !== activeThreadId.value) {
      recordFrontendDiagnostic(
        'thread.select',
        `from=${activeThreadId.value || '-'} to=${threadId}`,
      );
    }
    activeThreadId.value = threadId;
  }

  function clearSelection() {
    selectedThreadIds.value = [];
    selectionAnchorId.value = '';
  }

  function beginThreadLeave(threadId: string, destination: WorkflowState) {
    if (!leavingThreadIds.value.includes(threadId)) {
      leavingThreadIds.value = [...leavingThreadIds.value, threadId];
    }
    leavingThreadStates.value = {
      ...leavingThreadStates.value,
      [threadId]: destination,
    };
  }

  function endThreadLeave(threadId: string) {
    leavingThreadIds.value = leavingThreadIds.value.filter((id) => id !== threadId);
    const remaining = { ...leavingThreadStates.value };
    delete remaining[threadId];
    leavingThreadStates.value = remaining;
  }

  function setSelection(threadIds: string[]) {
    const unique = Array.from(new Set(threadIds));
    selectedThreadIds.value = unique.filter((id) => Boolean(find(id)));
  }

  function selectAllVisibleThreads() {
    const order = projection.visibleThreadOrder();
    setSelection(order);
    selectionAnchorId.value = order[0] ?? '';
  }

  function toggleThreadSelection(threadId: string) {
    if (selectedThreadSet.value.has(threadId)) {
      selectedThreadIds.value = selectedThreadIds.value.filter((id) => id !== threadId);
      return;
    }
    selectedThreadIds.value = [...selectedThreadIds.value, threadId];
  }

  function rangeSelection(fromThreadId: string, toThreadId: string): string[] {
    const order = projection.visibleThreadOrder();
    const fromIndex = order.indexOf(fromThreadId);
    const toIndex = order.indexOf(toThreadId);
    if (fromIndex < 0 || toIndex < 0) {
      return [toThreadId];
    }

    const start = Math.min(fromIndex, toIndex);
    const end = Math.max(fromIndex, toIndex);
    return order.slice(start, end + 1);
  }

  function selectFromInteraction(
    threadId: string,
    modifiers: SelectionModifiers,
    openThread: boolean,
  ) {
    const hasMultiModifier = modifiers.metaKey || modifiers.ctrlKey;

    if (!modifiers.shiftKey && !hasMultiModifier && openThread) {
      clearSelection();
    } else if (modifiers.shiftKey && selectionAnchorId.value) {
      const range = rangeSelection(selectionAnchorId.value, threadId);
      if (hasMultiModifier) {
        const next = new Set(selectedThreadIds.value);
        range.forEach((id) => next.add(id));
        setSelection([...next]);
      } else {
        setSelection(range);
      }
    } else if (hasMultiModifier) {
      toggleThreadSelection(threadId);
      selectionAnchorId.value = threadId;
    } else if (!openThread) {
      setSelection([threadId]);
      selectionAnchorId.value = threadId;
    }

    if (openThread) {
      activeThreadId.value = threadId;
    }
  }

  function toggleSelectionForThread(threadId: string) {
    toggleThreadSelection(threadId);
    selectionAnchorId.value = threadId;
  }

  function focusNeighborThread(threadId: string) {
    if (activeThreadId.value !== threadId) {
      return;
    }

    const order = projection.scopedVisibleThreads().map((thread) => thread.id);
    const index = order.indexOf(threadId);
    if (index < 0) {
      return;
    }

    activeThreadId.value = order[index + 1] ?? order[index - 1] ?? '';
  }

  function nextThread() {
    const visible = projection.scopedVisibleThreads();
    if (visible.length === 0) {
      return;
    }

    const currentIndex = visible.findIndex((thread) => thread.id === activeThreadId.value);
    const nextIndex = currentIndex < 0 ? 0 : Math.min(currentIndex + 1, visible.length - 1);
    activeThreadId.value = visible[nextIndex]?.id ?? '';
  }

  function prevThread() {
    const visible = projection.scopedVisibleThreads();
    if (visible.length === 0) {
      return;
    }

    const currentIndex = visible.findIndex((thread) => thread.id === activeThreadId.value);
    const prevIndex = currentIndex < 0 ? 0 : Math.max(currentIndex - 1, 0);
    activeThreadId.value = visible[prevIndex]?.id ?? '';
  }

  function reconcileRemovedThread(threadId: string) {
    selectedThreadIds.value = selectedThreadIds.value.filter((id) => id !== threadId);
    leavingThreadIds.value = leavingThreadIds.value.filter((id) => id !== threadId);
    delete leavingThreadStates.value[threadId];
    if (activeThreadId.value === threadId) {
      activeThreadId.value = projection.scopedVisibleThreads()[0]?.id ?? '';
    }
  }

  let watchersStarted = false;
  function startWatching() {
    if (watchersStarted) return;
    watchersStarted = true;

    watch(
      () => projection.scopedVisibleThreads(),
      (nextThreads) => {
        if (nextThreads.length === 0) {
          activeThreadId.value = '';
          return;
        }

        if (!nextThreads.some((thread) => thread.id === activeThreadId.value)) {
          activeThreadId.value = nextThreads[0]?.id ?? '';
        }
      },
      { immediate: true },
    );

    watch(() => projection.visibleThreadOrder(), (nextOrder) => {
      const orderSet = new Set(nextOrder);
      selectedThreadIds.value = selectedThreadIds.value.filter((id) => orderSet.has(id));
      if (selectionAnchorId.value && !orderSet.has(selectionAnchorId.value)) {
        selectionAnchorId.value = '';
      }
    });
  }

  return {
    surface: {
      activeThreadId,
      activeThread,
      selectedThreadIds,
      selectedThreads,
      leavingThreadIds,
      leavingThreadStates,
      selectThread,
      clearSelection,
      setSelection,
      selectAllVisibleThreads,
      selectFromInteraction,
      toggleSelectionForThread,
      focusNeighborThread,
      nextThread,
      prevThread,
      beginThreadLeave,
      endThreadLeave,
    },
    reconcileRemovedThread,
    startWatching,
  };
}
