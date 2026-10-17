import { computed } from 'vue';
import { useToast, type ToastAction } from '@/composables/useToast';

export interface UndoFabEntry {
  key: string;
  ids: string[];
  label: string;
  /** Hard wall clock: the first source to lapse takes its payload with it. */
  expiresAt: number;
  durationMs: number;
  count: number;
  sources: Array<{ id: string; action: ToastAction }>;
}

/**
 * Mirrors actionable toasts flagged `present: 'fab'` into the floating undo
 * control. The toast state stays the single source of truth — deadlines,
 * grouping and batch payloads live there; this only reshapes them for the
 * FAB presenter, exactly like ToastStack does for its own rows.
 */
export function useUndoFab() {
  const toast = useToast();

  const entries = computed<UndoFabEntry[]>(() => {
    const list: UndoFabEntry[] = [];
    for (const item of toast.state.toasts) {
      if (item.action?.present !== 'fab') continue;
      const key = item.groupKey ?? item.id;
      const index = list.findIndex((entry) => entry.key === key);
      if (index < 0) {
        list.push({
          key,
          ids: [item.id],
          label: item.message,
          expiresAt: item.expiresAt,
          durationMs: Math.max(item.timeout, 1),
          count: item.count ?? 1,
          sources: [{ id: item.id, action: item.action }],
        });
        continue;
      }
      const entry = list[index]!;
      entry.ids.push(item.id);
      entry.count += item.count ?? 1;
      entry.sources.push({ id: item.id, action: item.action });
      if (item.expiresAt < entry.expiresAt) {
        entry.expiresAt = item.expiresAt;
        entry.durationMs = Math.max(item.timeout, 1);
      }
    }
    return list;
  });

  /** Dismiss only sources whose handler accepted the request. */
  function run(entry: UndoFabEntry) {
    return toast.runActions(entry.ids, entry.sources.map((source) => source.action));
  }

  return { entries, run };
}
