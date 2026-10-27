import { computed, ref, shallowRef, watch } from 'vue';
import { backendCommandsAvailable, backendGetThreads } from '@/services/backend';
import { useSearch } from '@/composables/useSearch';
import { useAccountsStore } from '@/stores/accounts';
import { useListSearchStore } from '@/stores/listSearch';
import { useThreadsStore } from '@/stores/threads';
import {
  isVisibleInCurrentScope,
  matchesMailboxFilter,
  normalizedParticipantEmail,
  threadHasParticipant,
  toThreadModel,
  type MailboxFilter,
  type ThreadListEntry,
} from '@/utils/threadList';
import type { Thread } from '@/types/mail';
import type { WorkflowState } from '@/types/workflow';

/** The list the search narrows: what the thread store is showing. */
export interface ListSearchMailbox {
  accountId: string;
  workflowFilter: WorkflowState | 'all';
  mailboxFilter: MailboxFilter;
  activeContactEmail: string;
}

/** Hits per backend page. Larger than the full search's 50 because a mailbox
 *  scope may discard part of each page. */
export const LIST_SEARCH_PAGE_SIZE = 100;
/** A filtered search keeps paging on its own until it has this many rows… */
export const LIST_SEARCH_FILL_TARGET = 30;
/** …but stops after this many extra pages, so a common word in a small
 *  mailbox does not walk the whole archive. */
export const LIST_SEARCH_MAX_FILL_PAGES = 4;

/**
 * The backend operator that narrows a search to the mailbox, when one says
 * exactly that. `null` leaves the narrowing to `threadMatchesListSearch`:
 * the Inbox is `inbox` *and* `todo`, and custom folders, contacts,
 * notifications and blocked senders have no operator at all.
 */
export function mailboxSearchOperator(mailbox: ListSearchMailbox): string | null {
  if (mailbox.activeContactEmail) return null;
  const { workflowFilter, mailboxFilter } = mailbox;
  if (workflowFilter !== 'all') {
    return mailboxFilter.kind === 'all' || !mailboxFilter.value ? `in:${workflowFilter}` : null;
  }
  if (mailboxFilter.kind !== 'section') return null;
  switch (mailboxFilter.value) {
    case 'archive':
      return 'in:done';
    case 'snoozed':
    case 'aside':
    case 'sent':
    case 'drafts':
    case 'trash':
    case 'spam':
      return `in:${mailboxFilter.value}`;
    default:
      return null;
  }
}

/**
 * Whether a thread belongs to the list being searched — the same test the
 * thread store applies to its own page, minus the category tabs and the muted
 * filter: a search should find what those only tuck away.
 */
export function threadMatchesListSearch(thread: Thread, mailbox: ListSearchMailbox): boolean {
  if (mailbox.accountId && thread.accountId !== mailbox.accountId) return false;
  const contact = normalizedParticipantEmail(mailbox.activeContactEmail);
  if (contact) return threadHasParticipant(thread, contact);
  if (mailbox.workflowFilter !== 'all' && thread.workflowState !== mailbox.workflowFilter) return false;
  return matchesMailboxFilter(thread, mailbox.mailboxFilter)
    && isVisibleInCurrentScope(thread, mailbox.workflowFilter, mailbox.mailboxFilter);
}

/** A typed `in:` names its own place; the mailbox scope then steps aside. */
export function namesLocation(query: string): boolean {
  return /(^|\s)in:\S/i.test(query);
}

/**
 * Searches the list on screen, or all mail, and hands back thread rows.
 *
 * The query runs through the same FTS search as the search page — operators
 * included — and the hits are resolved to thread models so they render as the
 * Inbox's own rows. A row reads the store's copy when there is one, so an
 * archive or a star from the list shows straight away and a thread that left
 * the mailbox drops out of a mailbox-scoped search.
 */
export function useListSearch(options: { mailbox: () => ListSearchMailbox }) {
  const store = useListSearchStore();
  const accounts = useAccountsStore();
  const threads = useThreadsStore();

  const narrowed = computed(() => store.scope === 'mailbox' && !namesLocation(store.query));
  // "All mail" means every account too, like the scope bar in Apple Mail.
  const accountFilter = computed(() => (store.scope === 'all' ? '' : options.mailbox().accountId));

  const search = useSearch({ accountFilter, liveUpdates: true, limit: LIST_SEARCH_PAGE_SIZE });

  const effectiveQuery = computed(() => {
    const raw = store.query.trim();
    if (!raw) return '';
    const operator = narrowed.value ? mailboxSearchOperator(options.mailbox()) : null;
    return operator ? `${operator} ${raw}` : raw;
  });
  watch(effectiveQuery, (query) => { search.query.value = query; }, { immediate: true, flush: 'sync' });

  // Hits name threads; the list needs the threads. Fetched once per id and
  // kept while the search lives, so a row does not vanish when the store
  // trims its own page underneath.
  const fetched = shallowRef(new Map<string, Thread>());
  const fetching = ref(false);
  let fetchesInFlight = 0;

  watch(search.results, async (rows) => {
    const missing = [...new Set(rows.map((row) => row.threadId))]
      .filter((id) => !fetched.value.has(id));
    if (missing.length === 0 || !backendCommandsAvailable()) return;
    fetchesInFlight += 1;
    fetching.value = true;
    try {
      const dtos = await backendGetThreads(missing);
      const next = new Map(fetched.value);
      for (const dto of dtos) next.set(dto.id, toThreadModel(dto, accounts.accounts));
      fetched.value = next;
    } catch {
      // A hit without a thread stays out of the list; the next query retries.
    } finally {
      fetchesInFlight -= 1;
      fetching.value = fetchesInFlight > 0;
    }
  });

  const storeThreads = computed(() => new Map(threads.threads.map((thread) => [thread.id, thread])));

  const resultThreads = computed<Thread[]>(() => {
    const mailbox = options.mailbox();
    const seen = new Set<string>();
    const rows: Thread[] = [];
    for (const hit of search.results.value) {
      if (seen.has(hit.threadId)) continue;
      seen.add(hit.threadId);
      const thread = storeThreads.value.get(hit.threadId) ?? fetched.value.get(hit.threadId);
      if (!thread) continue;
      if (narrowed.value && !threadMatchesListSearch(thread, mailbox)) continue;
      rows.push(thread);
    }
    return rows;
  });

  const entries = computed<ThreadListEntry[]>(() =>
    resultThreads.value.map((thread) => ({ kind: 'thread', id: `thread-${thread.id}`, thread })),
  );

  // A narrowed search may keep only part of each page. Fill the list a few
  // pages deep on its own; past that the scope switch is the way on.
  let fillPages = 0;
  watch(effectiveQuery, () => { fillPages = 0; });
  watch(accountFilter, () => { fillPages = 0; });
  const fillExhausted = ref(false);
  watch(
    [resultThreads, search.hasMore, search.loading, search.loadingMore, fetching],
    () => {
      fillExhausted.value = false;
      if (!narrowed.value || !search.hasMore.value) return;
      if (search.loading.value || search.loadingMore.value || fetching.value) return;
      if (resultThreads.value.length >= LIST_SEARCH_FILL_TARGET) return;
      if (fillPages >= LIST_SEARCH_MAX_FILL_PAGES) {
        fillExhausted.value = true;
        return;
      }
      fillPages += 1;
      void search.loadMore();
    },
  );

  /**
   * The list asks for more near its end — and also whenever it is shorter
   * than the viewport. A narrowed, still short list is the filler's job;
   * answering here as well would page through the whole archive.
   */
  function loadMore() {
    if (narrowed.value && resultThreads.value.length < LIST_SEARCH_FILL_TARGET) return;
    void search.loadMore();
  }

  const loading = computed(() =>
    resultThreads.value.length === 0
      && (search.loading.value || fetching.value || (narrowed.value && search.hasMore.value && !fillExhausted.value)),
  );

  return {
    entries,
    resultThreads,
    loading,
    showProgress: search.showLoading,
    loadingMore: search.loadingMore,
    hasMore: search.hasMore,
    errorText: search.errorText,
    warnings: search.warnings,
    pendingCount: search.pendingCount,
    refresh: search.refresh,
    narrowed,
    loadMore,
  };
}
