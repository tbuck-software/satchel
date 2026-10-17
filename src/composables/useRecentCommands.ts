import { ref } from 'vue';
import { MAX_RECENT_COMMANDS } from '@/config/historyLimits';
import { searchPersistence } from '@/persistence/search';

const recentIds = ref<string[]>(searchPersistence.recentCommands.read());

/** Palette commands by last use, most recent first; ids only, titles change with state. */
export function useRecentCommands() {
  function record(id: string) {
    recentIds.value = [id, ...recentIds.value.filter((entry) => entry !== id)].slice(0, MAX_RECENT_COMMANDS);
    searchPersistence.recentCommands.write(recentIds.value);
  }

  return { recentIds, record };
}
