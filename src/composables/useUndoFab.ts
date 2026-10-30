import { computed } from 'vue';
import { useUndo } from './useUndo';

export type { UndoEntry as UndoFabEntry } from './useUndo';

/** Show FAB receipts while shortcut hints follow the latest Undo in either presenter. */
export function useUndoFab() {
  const fab = useUndo('fab');
  const all = useUndo();
  const latestKey = computed(() => all.entries.value.at(-1)?.key);
  return { ...fab, latestKey };
}
