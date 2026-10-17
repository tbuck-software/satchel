import { ref } from 'vue';
import { MAX_RECENT_SEARCHES } from '@/config/historyLimits';
import { searchPersistence } from '@/persistence/search';

// Module-level so the omni panel and the search page share one list.
const recent = ref<string[]>(searchPersistence.recentQueries.read());

function persist() {
  searchPersistence.recentQueries.write(recent.value);
}

/**
 * Recently executed search queries, most recent first. Fed by the omni panel
 * and the search page; shown in the omni panel while the query is empty.
 */
export function useRecentSearches() {
  function push(query: string) {
    const trimmed = query.trim();
    if (!trimmed) return;
    const needle = trimmed.toLowerCase();
    recent.value = [
      trimmed,
      ...recent.value.filter((entry) => entry.toLowerCase() !== needle),
    ].slice(0, MAX_RECENT_SEARCHES);
    persist();
  }

  function remove(query: string) {
    const needle = query.trim().toLowerCase();
    recent.value = recent.value.filter((entry) => entry.toLowerCase() !== needle);
    persist();
  }

  function clear() {
    recent.value = [];
    persist();
  }

  return { recent, push, remove, clear };
}
