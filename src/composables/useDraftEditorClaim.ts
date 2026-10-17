import { onBeforeUnmount, watch, type Ref } from 'vue';
import {
  backendClaimDraftEditor,
  backendCommandsAvailable,
  backendReleaseDraftEditor,
} from '@/services/backend';

/**
 * Registers this composer as the one window editing its draft. When another
 * window already has it, the backend brings that window forward and
 * `onRefused` lets this composer step aside.
 */
export function useDraftEditorClaim(
  draftId: Ref<string | null>,
  options: { onRefused: () => void },
) {
  let claimed: string | null = null;

  async function claim(id: string | null) {
    if (!id || id === claimed || !backendCommandsAvailable()) return;
    try {
      if (await backendClaimDraftEditor(id)) claimed = id;
      else options.onRefused();
    } catch (error) {
      console.warn('draft editor claim failed', error);
    }
  }

  /** The draft has left this editor (sent, discarded, popped out). */
  function release() {
    const id = claimed;
    claimed = null;
    if (!id) return;
    backendReleaseDraftEditor(id).catch((error: unknown) => {
      console.warn('draft editor release failed', error);
    });
  }

  watch(draftId, (id) => void claim(id), { immediate: true });
  onBeforeUnmount(release);

  return { release };
}
