import { computed } from 'vue';
import { useToast, type ToastAction } from '@/composables/useToast';

export interface UndoEntry {
  key: string;
  ids: string[];
  label: string;
  /** Hard wall clock: the first source to lapse takes its payload with it. */
  expiresAt: number;
  durationMs: number;
  count: number;
  sources: Array<{ id: string; action: ToastAction }>;
}

/** Group Undo receipts independently of their visual presenter. */
export function useUndo(presentation?: 'fab') {
  const toast = useToast();

  const entries = computed<UndoEntry[]>(() => {
    const list: UndoEntry[] = [];
    for (const item of toast.state.toasts) {
      if (!item.action || (!item.action.undo && item.action.present !== 'fab')) continue;
      if (presentation && item.action.present !== presentation) continue;
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
  function run(entry: UndoEntry) {
    return toast.runActions(entry.ids, entry.sources.map((source) => source.action));
  }

  return { entries, run };
}
