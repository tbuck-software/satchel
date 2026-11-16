import { defineStore } from 'pinia';
import { ref } from 'vue';
import {
  backendCommandsAvailable,
  backendListLabsFlags,
  backendSetLabsFlag,
} from '@/services/backend';
import { LABS_FLAGS, LABS_FLAG_IDS, isLabsFlagId, type LabsFlagId } from '@/labs/flags';
import { orderedLoad } from './internal/orderedLoad';

function defaults(): Record<LabsFlagId, boolean> {
  return Object.fromEntries(
    LABS_FLAG_IDS.map((id) => [id, LABS_FLAGS[id].defaultEnabled]),
  ) as Record<LabsFlagId, boolean>;
}

/**
 * Labs flags on this device: the one place the app asks whether an
 * experiment is on.
 *
 * Until the backend answers every flag reads as its default (off), so an
 * experiment never flashes in and out on start. `ready` settles after the
 * first load, for code that must not decide before it (a deep link into a
 * Labs view). Only lazy code imports this store; startup never loads it.
 */
export const useLabsStore = defineStore('labs', () => {
  const enabled = ref<Record<LabsFlagId, boolean>>(defaults());
  /** The switch the backend has not confirmed yet. */
  const saving = ref<LabsFlagId | null>(null);
  const loader = orderedLoad();
  const { loading, loadFailed } = loader;
  let settle: () => void = () => {};
  const ready = new Promise<void>((resolve) => { settle = resolve; });
  let started: Promise<void> | null = null;

  async function load() {
    if (!backendCommandsAvailable()) {
      settle();
      return;
    }
    await loader.load(backendListLabsFlags, (rows) => {
      const next = defaults();
      for (const row of rows) {
        if (isLabsFlagId(row.id)) next[row.id] = row.enabled;
      }
      enabled.value = next;
    });
    settle();
  }

  /** The first load, or the one already under way. */
  function loadOnce(): Promise<void> {
    started ??= load();
    return started;
  }

  function isEnabled(id: LabsFlagId): boolean {
    return enabled.value[id];
  }

  /** Switches at once and puts the old state back if the backend refuses. */
  async function setEnabled(id: LabsFlagId, value: boolean): Promise<void> {
    const previous = enabled.value[id];
    enabled.value = { ...enabled.value, [id]: value };
    saving.value = id;
    try {
      const row = await loader.mutate(() => backendSetLabsFlag(id, value));
      enabled.value = { ...enabled.value, [id]: row.enabled };
    } catch (error) {
      enabled.value = { ...enabled.value, [id]: previous };
      throw error;
    } finally {
      if (saving.value === id) saving.value = null;
    }
  }

  return { enabled, loading, loadFailed, saving, ready, load, loadOnce, isEnabled, setEnabled };
});
