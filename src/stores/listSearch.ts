import { computed, ref } from 'vue';
import { defineStore } from 'pinia';

/** `mailbox` searches the list on screen, `all` every mailbox of every account. */
export type ListSearchScope = 'mailbox' | 'all';

/**
 * The search above the thread list.
 *
 * Only the request lives here — query, scope and which result is open — so
 * the sidebar, the palette and the keyboard can start a search without
 * loading the search machinery. The results component owns the backend work
 * and is loaded on the first keystroke. A store rather than view state so a
 * search survives the round trip through the thread route on a phone.
 */
export const useListSearchStore = defineStore('listSearch', () => {
  const query = ref('');
  const scope = ref<ListSearchScope>('mailbox');
  /** The result shown in the reading pane; '' leaves it to the first hit. */
  const openThreadId = ref('');
  /** Result order, for the reader's position and previous/next. */
  const resultThreadIds = ref<string[]>([]);

  const active = computed(() => query.value.trim().length > 0);

  /** Start a search from elsewhere: a contact, a saved search, the palette. */
  function open(next: string, nextScope: ListSearchScope = 'all') {
    query.value = next;
    scope.value = nextScope;
    openThreadId.value = '';
  }

  function setQuery(next: string) {
    query.value = next;
  }

  function setScope(next: ListSearchScope) {
    scope.value = next;
  }

  function clear() {
    query.value = '';
    scope.value = 'mailbox';
    openThreadId.value = '';
    resultThreadIds.value = [];
  }

  return { query, scope, openThreadId, resultThreadIds, active, open, setQuery, setScope, clear };
});
