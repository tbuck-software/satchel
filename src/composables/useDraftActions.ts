import { ref } from 'vue';
import { t } from '@/i18n/runtime';
import type { BackendDraft } from '@/services/backend';
import { useDraftsStore } from '@/stores/drafts';
import { useToast } from '@/composables/useToast';

/** Matches the undo window the thread actions use. */
const UNDO_TIMEOUT_MS = 9000;

function reason(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/**
 * Discarding a draft, with the same undo affordance everywhere it can happen.
 *
 * A draft lives in two places at once — the drafts list and, once it belongs to
 * a conversation, inside that thread. Both need identical behaviour, down to
 * the toast, so neither becomes the one that quietly deletes without recourse.
 */
export function useDraftActions() {
  const drafts = useDraftsStore();
  const toast = useToast();

  /** Ids with a running delete — keeps a double click from firing twice. */
  const busy = ref(new Set<string>());

  function isBusy(draftId: string): boolean {
    return busy.value.has(draftId);
  }

  function markBusy(draftId: string, value: boolean) {
    const next = new Set(busy.value);
    if (value) next.add(draftId);
    else next.delete(draftId);
    busy.value = next;
  }

  /**
   * Undo needs the draft's content, and after the delete there is nothing left
   * to read it from — so it is read first, from the store when the list is
   * loaded and from the backend otherwise.
   */
  async function snapshot(draftId: string): Promise<BackendDraft | null> {
    const known = drafts.items.find((item) => item.id === draftId);
    if (known) return known;
    try {
      return await drafts.fetchOne(draftId);
    } catch {
      return null;
    }
  }

  async function restore(draft: BackendDraft) {
    try {
      await drafts.restore(draft);
      toast.success(t('drafts.restored'));
    } catch (err) {
      toast.error(t('drafts.restoreFailed', { error: reason(err) }));
    }
  }

  /** Resolves to `false` when the draft was left in place. */
  async function discard(draftId: string): Promise<boolean> {
    if (!draftId || busy.value.has(draftId)) return false;
    markBusy(draftId, true);

    const previous = await snapshot(draftId);
    try {
      await drafts.remove(draftId);
    } catch (err) {
      toast.error(t('drafts.deleteFailed', { error: reason(err) }));
      return false;
    } finally {
      markBusy(draftId, false);
    }

    // Without a snapshot there is nothing to restore, so no dead undo button.
    toast.info(t('drafts.deleted'), {
      timeout: UNDO_TIMEOUT_MS,
      action: previous
        ? { label: t('drafts.undo'), handler: () => void restore(previous) }
        : undefined,
    });
    return true;
  }

  return { busy, isBusy, discard };
}
