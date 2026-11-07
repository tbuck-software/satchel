import { watch } from 'vue';
import { LABS_FLAG_IDS, type LabsFlagId } from './flags';

/** What an experiment does while its flag is on; the return value undoes it. */
export interface LabsActivation {
  activate(): () => void;
}

/**
 * The code each flag turns on, behind a dynamic import. With the flag off the
 * module is never fetched: no code and no strings of the experiment load at
 * startup (ADR 0017).
 */
const ACTIVATIONS: Record<LabsFlagId, () => Promise<LabsActivation>> = {
  chats: () => import('@/components/chats/activateChats'),
};

/**
 * Runs each experiment's activation while its flag is on and undoes it when
 * the flag goes off. Returns a stop function for the app's teardown.
 */
export function startLabsActivation(isEnabled: (id: LabsFlagId) => boolean): () => void {
  const active = new Map<LabsFlagId, () => void>();
  const stops = LABS_FLAG_IDS.map((id) => {
    let generation = 0;
    return watch(() => isEnabled(id), async (on) => {
      const current = ++generation;
      if (!on) {
        active.get(id)?.();
        active.delete(id);
        return;
      }
      if (active.has(id)) return;
      try {
        const activation = await ACTIVATIONS[id]();
        if (current !== generation || !isEnabled(id) || active.has(id)) return;
        active.set(id, activation.activate());
      } catch (error) {
        console.warn(`labs: ${id} failed to activate`, error);
      }
    }, { immediate: true });
  });
  return () => {
    for (const stop of stops) stop();
    for (const deactivate of active.values()) deactivate();
    active.clear();
  };
}
