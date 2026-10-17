import { t } from '@/i18n/runtime';
import { useToast } from '@/composables/useToast';
import { backendErrorLabel } from '@/utils/backendErrorLabels';

/** Matches the undo window the thread actions use. */
const UNDO_TIMEOUT_MS = 9000;

export interface CrudRemoval<T> {
  /** The entry's id — keeps a double click from deleting twice. */
  id: string;
  /** Names the entry in every message, so it reads like the row that was hit. */
  name: string;
  /**
   * Deletes the entry and resolves with the snapshot Undo needs. Resolving
   * with `null` means there was nothing to snapshot, and no Undo is offered.
   */
  remove: () => Promise<T | null>;
  /** Puts the snapshot back through the store's regular upsert path. */
  restore: (entry: T) => Promise<unknown>;
}

/**
 * Deleting a settings entry — signature, alias, template, smart folder — in the
 * one order that neither loses the entry nor lies about the outcome.
 *
 * The four editors used to call their store straight out of `@click`, with no
 * `catch`: a rejected delete left the row standing and said nothing, so a
 * failure looked exactly like a success. They also had no way back, while
 * `AGENTS.md` promises Undo on every destructive action.
 *
 * Nothing here is optimistic. The stores drop the entry only after the backend
 * confirmed the delete, the same order `useNewSenderActions` holds for blocking
 * a sender: a list that already forgot an entry the backend still has is the
 * worse of the two wrong states, because nothing ever asks about it again.
 */
export function useCrudActions() {
  const toast = useToast();

  /** Ids with a delete in flight, so a double click cannot delete twice. */
  const busy = new Set<string>();

  async function restore<T>(removal: CrudRemoval<T>, entry: T) {
    try {
      await removal.restore(entry);
      toast.success(t('settings.entries.restored', { name: removal.name }));
    } catch (error) {
      toast.error(
        t('settings.entries.restoreFailed', {
          name: removal.name,
          error: backendErrorLabel(error),
        }),
      );
    }
  }

  /** Resolves to `false` when the entry was left in place. */
  async function removeWithUndo<T>(removal: CrudRemoval<T>): Promise<boolean> {
    if (busy.has(removal.id)) return false;
    busy.add(removal.id);

    let snapshot: T | null = null;
    try {
      snapshot = await removal.remove();
    } catch (error) {
      toast.error(
        t('settings.entries.deleteFailed', {
          name: removal.name,
          error: backendErrorLabel(error),
        }),
      );
      return false;
    } finally {
      busy.delete(removal.id);
    }

    const restorable = snapshot;
    toast.info(t('settings.entries.deleted', { name: removal.name }), {
      timeout: UNDO_TIMEOUT_MS,
      action: restorable
        ? { label: t('common.undo'), handler: () => void restore(removal, restorable) }
        : undefined,
    });
    return true;
  }

  return { removeWithUndo };
}
