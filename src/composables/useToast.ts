import { reactive } from 'vue';
import { t } from '@/i18n/runtime';

export type ToastTone = 'info' | 'success' | 'warning' | 'error';

export interface ToastAction {
  label: string;
  /** Returning false refuses the action and keeps its receipt. */
  handler: () => unknown | Promise<unknown>;
  /**
   * Render this action in the undo FAB instead of as an inline toast button.
   * The toast itself stays the single source of truth — deadline, grouping
   * and batch payload live here; the FAB only mirrors them.
   */
  present?: 'fab';
  /** Include an inline action in global Undo shortcuts and the command palette. */
  undo?: boolean;
  /** Combine equivalent Undo payloads into one persistence/refresh pass. */
  batch?: {
    key: string;
    items: unknown[];
    handler: (items: unknown[]) => unknown | Promise<unknown>;
  };
}

export interface Toast {
  id: string;
  message: string;
  tone: ToastTone;
  /** Auto-dismiss timeout in ms; 0 = persistent. */
  timeout: number;
  action?: ToastAction;
  /** Visually combine equivalent actions while retaining independent expiry. */
  groupKey?: string;
  /** Number of affected items represented by this toast. */
  count?: number;
  /** Recompute the visible copy when grouped toast counts change. */
  groupMessage?: (count: number) => string;
  /**
   * When the toast disappears. Held explicitly rather than derived from the id
   * so it can be pushed back while someone is reading — an undo action is
   * useless if it expires mid-sentence.
   */
  expiresAt: number;
  /** Set while a non-actionable toast is hovered or focused. */
  pausedAt: number | null;
}

const state = reactive({
  toasts: [] as Toast[],
});

let timer: ReturnType<typeof setInterval> | null = null;

function stopTicker() {
  if (!timer) return;
  clearInterval(timer);
  timer = null;
}

function ensureTicker() {
  // Guards on what is actually used. The previous check was for `window`, which
  // meant the countdown silently never ran anywhere without a DOM — including
  // under vitest, where this file's behaviour could not be tested at all.
  if (timer || typeof setInterval === 'undefined') return;
  timer = setInterval(() => {
    const now = Date.now();
    state.toasts = state.toasts.filter(
      (toast) => toast.timeout <= 0 || toast.pausedAt !== null || now < toast.expiresAt,
    );
    if (state.toasts.length === 0) stopTicker();
  }, 250);
}

const runningActions = new Set<string>();
let counter = 0;

/**
 * Whether an equivalent toast can absorb this one.
 *
 * Actionable toasts only merge when both carry the same Undo batch: keeping
 * one of two unrelated handlers would silently drop the other's payload.
 */
function absorbs(existing: Toast, incoming: Partial<Toast>): boolean {
  if (!existing.groupKey || existing.groupKey !== incoming.groupKey) return false;
  if (!existing.action && !incoming.action) return true;
  const batch = existing.action?.batch;
  const next = incoming.action?.batch;
  return Boolean(batch && next && batch.key === next.key);
}

function push(message: string, tone: ToastTone, options: Partial<Toast> = {}) {
  const timeout = options.timeout ?? (tone === 'error' ? 9000 : 4500);
  const count = options.count ?? 1;

  if (options.groupKey) {
    const existing = state.toasts.find((entry) => !runningActions.has(entry.id) && absorbs(entry, options));
    if (existing) {
      existing.count = (existing.count ?? 1) + count;
      existing.message = options.groupMessage?.(existing.count) ?? message;
      const batch = existing.action?.batch;
      const incoming = options.action?.batch;
      if (batch && incoming) batch.items = [...batch.items, ...incoming.items];
      // An actionable group runs on the earliest deadline of its sources: Undo
      // is only runnable until the backend picks the queued move up, so a
      // group that keeps growing must not outlive the window it promises.
      existing.expiresAt = existing.action
        ? Math.min(existing.expiresAt, Date.now() + timeout)
        : Math.max(existing.expiresAt, Date.now() + timeout);
      // A growing receipt is the newest action even if its group existed earlier.
      state.toasts = [...state.toasts.filter((entry) => entry.id !== existing.id), existing];
      ensureTicker();
      return existing.id;
    }
  }

  counter += 1;
  const toast: Toast = {
    id: `toast-${counter}`,
    message,
    tone,
    timeout,
    action: options.action,
    groupKey: options.groupKey,
    count: options.count,
    groupMessage: options.groupMessage,
    expiresAt: Date.now() + timeout,
    pausedAt: null,
  };
  state.toasts.push(toast);
  ensureTicker();
  return toast.id;
}

/** Shared by both receipt presenters; one invocation owns each source at a time. */
async function runActions(ids: string[], actions: ToastAction[]): Promise<boolean> {
  if (ids.some((id) => runningActions.has(id)) || actions.length === 0) return false;
  ids.forEach((id) => runningActions.add(id));
  try {
    const batch = actions[0]?.batch;
    if (batch && actions.every((action) => action.batch?.key === batch.key)) {
      if (await batch.handler(actions.flatMap((action) => action.batch?.items ?? [])) === false) return false;
      ids.forEach((id) => useToast().dismiss(id));
    } else {
      for (const [index, action] of actions.entries()) {
        if (await action.handler() === false) return false;
        const id = ids[index];
        if (id) useToast().dismiss(id);
      }
    }
    return true;
  } catch {
    useToast().error(t('backend.errors.mailActionFailed'));
    return false;
  } finally {
    ids.forEach((id) => runningActions.delete(id));
  }
}

export function useToast() {
  return {
    state,
    runActions,
    info: (msg: string, opts?: Partial<Toast>) => push(msg, 'info', opts),
    success: (msg: string, opts?: Partial<Toast>) => push(msg, 'success', opts),
    warning: (msg: string, opts?: Partial<Toast>) => push(msg, 'warning', opts),
    error: (msg: string, opts?: Partial<Toast>) => push(msg, 'error', opts),
    /** Freeze the countdown — called when a toast is hovered or focused. */
    pause(id: string) {
      const toast = state.toasts.find((item) => item.id === id);
      // Actionable Undo expires on a hard wall clock before its durable
      // provider operation becomes runnable. Extending it would leave a
      // visible button that can no longer cancel what it promises to undo.
      if (toast && !toast.action && toast.pausedAt === null) toast.pausedAt = Date.now();
    },
    /** Resume, giving back exactly the time that was left when it paused. */
    resume(id: string) {
      const toast = state.toasts.find((item) => item.id === id);
      if (!toast || toast.pausedAt === null) return;
      toast.expiresAt += Date.now() - toast.pausedAt;
      toast.pausedAt = null;
    },
    dismiss(id: string) {
      state.toasts = state.toasts.filter((t) => t.id !== id);
    },
    dismissGroup(groupKey: string) {
      state.toasts = state.toasts.filter((t) => t.groupKey !== groupKey);
      if (state.toasts.length === 0) stopTicker();
    },
    clear() {
      state.toasts = [];
      stopTicker();
    },
  };
}
