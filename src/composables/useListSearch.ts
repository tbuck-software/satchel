import { computed, ref, shallowRef, watch } from 'vue';
import {
  backendCommandsAvailable,
  backendGetThreads,
  type BackendSearchListScope,
} from '@/services/backend';
import { useSearch } from '@/composables/useSearch';
import { threadListInput } from '@/stores/threads/load-state';
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
import { matchesWorkflowFilter, type WorkflowFilter } from '@/types/workflow';

/** The list the search narrows: what the thread store is showing. */
export interface ListSearchMailbox {
  accountId: string;
  workflowFilter: WorkflowFilter;
  mailboxFilter: MailboxFilter;
  activeContactEmail: string;
}

/** Hits per backend page. */
export const LIST_SEARCH_PAGE_SIZE = 100;

/**
 * The list a narrowed search runs in, in the words of that list's own
 * request: the backend tests each hit's thread exactly as it builds the list
 * — the Inbox's inbox-and-todo threads with mail in the inbox folder, a
 * custom folder, a contact — so no page of hits is thrown away here.
 */
export function listSearchScope(mailbox: ListSearchMailbox): BackendSearchListScope {
  const input = threadListInput({ ...mailbox, activeSmartBucketId: '' }, 0);
  return {
    contact_email: input.contact_email ?? null,
    mailbox_kind: input.mailbox_kind ?? null,
    mailbox_value: input.mailbox_value ?? null,
    workflow_state: input.workflow_state ?? null,
  };
}

/**
 * Whether a thread still belongs to the list being searched — the same test
 * the thread store applies to its own page, minus the category tabs and the
 * muted filter: a search should find what those only tuck away. The backend
 * has already narrowed the hits; this drops a row whose store copy has since
 * left the list, an archive from the results for one.
 */
export function threadMatchesListSearch(thread: Thread, mailbox: ListSearchMailbox): boolean {
  if (mailbox.accountId && thread.accountId !== mailbox.accountId) return false;
  const contact = normalizedParticipantEmail(mailbox.activeContactEmail);
  if (contact) return threadHasParticipant(thread, contact);
  if (!matchesWorkflowFilter(thread.workflowState, mailbox.workflowFilter)) return false;
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

  const listScope = computed(() => (narrowed.value ? listSearchScope(options.mailbox()) : null));

  const search = useSearch({ accountFilter, listScope, liveUpdates: true, limit: LIST_SEARCH_PAGE_SIZE });
  watch(() => store.query.trim(), (query) => { search.query.value = query; }, { immediate: true, flush: 'sync' });

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

  const loading = computed(() =>
    resultThreads.value.length === 0 && (search.loading.value || fetching.value),
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
    loadMore: search.loadMore,
  };
}
