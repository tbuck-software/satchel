import { computed, getCurrentInstance, onBeforeUnmount, ref, watch, type Ref } from 'vue';
import {
  backendSearchMessageHits,
  backendCommandsAvailable,
  type BackendMessageSearchHit,
  type BackendSearchWarning,
  type BackendMessageSearchResponse,
  type BackendSearchListScope,
} from '@/services/backend';
import { useAccountsStore } from '@/stores/accounts';
import { useMailEvents } from '@/composables/useMailEvents';
import { splitHighlight, type HighlightSegment } from '@/utils/highlight';
import { t } from '@/i18n/runtime';

export interface SearchResult {
  messageId: string;
  threadId: string;
  subject: string;
  snippet: string;
  /** Snippet split into plain and matched ranges — render via interpolation. */
  snippetSegments: HighlightSegment[];
  fromName: string;
  fromAddress: string;
  date: string;
  isRead: boolean;
  /** Translated label for where the hit lives; an empty string means inbox. */
  location: string;
}

// Local FTS is cheap enough that 220ms felt like input lag. 120ms still
// coalesces normal typing without making the interface wait for the user.
export const SEARCH_DEBOUNCE_MS = 120;
// Fast searches should look instant, not flash a spinner for a few frames.
const LOADING_INDICATOR_DELAY_MS = 160;

/**
 * Where a hit lives, as a short badge label. The folder wins over the
 * workflow state — a mail in trash is "Papierkorb" no matter what its
 * thread's state says. Archiving equals the done state in Satchel, so
 * done reads "Erledigt".
 */
function locationLabel(folderType: string, workflowState: string): string {
  switch (folderType) {
    case 'trash':
      return t('folders.trash');
    case 'spam':
      return t('folders.spam');
    case 'sent':
      return t('folders.sent');
    case 'drafts':
      return t('folders.draft');
    case 'archive':
      return t('thread.state.done');
    default:
      break;
  }
  switch (workflowState) {
    case 'done':
      return t('thread.state.done');
    case 'snoozed':
      return t('thread.state.snoozed');
    case 'aside':
      return t('thread.state.aside');
    case 'waiting':
      return t('thread.state.waiting');
    case 'todo':
      return t('folders.todo');
    default:
      return '';
  }
}

function warningText(warning: BackendSearchWarning): string {
  if (warning.operator === 'before' || warning.operator === 'after') {
    return t('search.warning.date', { operator: warning.operator, value: warning.value });
  }
  return t('search.warning.unknown', { operator: warning.operator, value: warning.value });
}

/**
 * Full-text search over every stored message, backed by SQLite FTS5.
 *
 * The operator syntax (`from:`, `to:`, `cc:`, `subject:`, `in:`, `tag:`,
 * `has:attachment`, `is:unread`, `is:starred`, `before:`, `after:`, `-term`
 * for exclusion) is parsed
 * exclusively in `commands/messages.rs::parse_search_query`. There used to be a
 * second, weaker parser here that filtered only the already-loaded threads over
 * subject+snippet — it never searched message bodies and silently reported
 * "no results" for mail that existed. Do not reintroduce client-side parsing;
 * pass the raw query through so both entry points behave identically.
 */
export function useSearch(options?: {
  /**
   * Explicit account filter: '' searches every account, a non-empty id only
   * that one. Without the option the active account scopes the search (the
   * omni panel's "search in current context" behaviour). The full search
   * page passes this and renders it as a removable filter chip.
   */
  accountFilter?: Ref<string>;
  /**
   * Re-run the query in the background when new mail arrives and expose the
   * difference through `pendingCount`. Off by default: the omni panel is
   * short-lived and would only spend queries on results nobody reads.
   */
  liveUpdates?: boolean;
  /** Bounded preview size; full search uses the default page size. */
  limit?: number;
  /**
   * Only hits whose thread is in this list, as the list itself decides —
   * the list search's "this mailbox". `null` searches everywhere.
   */
  listScope?: Ref<BackendSearchListScope | null>;
}) {
  const accountsStore = useAccountsStore();
  const accountFilter = options?.accountFilter;

  function requestOptions() {
    const listScope = options?.listScope?.value;
    return {
      accountId: effectiveAccountId(),
      ...(options?.limit ? { limit: options.limit } : {}),
      ...(listScope ? { listScope } : {}),
    };
  }

  function effectiveAccountId(): string | undefined {
    if (accountFilter) return accountFilter.value || undefined;
    return accountsStore.activeAccountId || undefined;
  }

  const query = ref('');
  const results = ref<SearchResult[]>([]);
  const loading = ref(false);
  const loadingMore = ref(false);
  const hasMore = ref(false);
  let cursor: BackendMessageSearchResponse['next_cursor'] = null;
  let pendingCursor: BackendMessageSearchResponse['next_cursor'] = null;
  let pendingHasMore = false;
  const showLoading = ref(false);
  const errorText = ref('');
  /** Translated notes about ignored operators, e.g. an unknown `in:` value. */
  const warnings = ref<string[]>([]);
  /**
   * Result set fetched after new mail arrived, held back until the user asks
   * for it. Showing it straight away would resort the list under the cursor
   * while they are reading a hit.
   */
  const pendingResults = ref<SearchResult[] | null>(null);
  const pendingCount = ref(0);

  let debounceHandle: ReturnType<typeof setTimeout> | undefined;
  let liveHandle: ReturnType<typeof setTimeout> | undefined;
  let loadingIndicatorHandle: ReturnType<typeof setTimeout> | undefined;
  let requestId = 0;

  let searching = false;
  let queuedSearch: (() => Promise<void>) | undefined;

  // Keep one request in flight and replace queued work with the newest query.
  async function scheduleSearch(raw: string, id: number, live = false) {
    if (live && loading.value) return;
    const job = async () => {
      if (id !== requestId) return;
      if (live) await runLive(id);
      else await run(raw, id);
    };
    if (searching) {
      queuedSearch = job;
      return;
    }
    searching = true;
    try {
      await job();
      while (queuedSearch) {
        const next = queuedSearch;
        queuedSearch = undefined;
        await next();
      }
    } finally {
      searching = false;
    }
  }

  const trimmedQuery = computed(() => query.value.trim());
  const hasQuery = computed(() => trimmedQuery.value.length > 0);

  function beginLoading() {
    loading.value = true;
    showLoading.value = false;
    if (loadingIndicatorHandle) clearTimeout(loadingIndicatorHandle);
    loadingIndicatorHandle = setTimeout(() => {
      loadingIndicatorHandle = undefined;
      showLoading.value = loading.value;
    }, LOADING_INDICATOR_DELAY_MS);
  }

  function endLoading() {
    loading.value = false;
    showLoading.value = false;
    if (loadingIndicatorHandle) {
      clearTimeout(loadingIndicatorHandle);
      loadingIndicatorHandle = undefined;
    }
  }

  function toResult(hit: BackendMessageSearchHit): SearchResult {
    const segments = splitHighlight(hit.snippet || '');
    return {
      messageId: hit.id,
      threadId: hit.thread_id,
      subject: hit.subject || t('search.noSubject'),
      snippet: segments.map((segment) => segment.text).join(''),
      snippetSegments: segments,
      fromName: hit.from_name || hit.from_address,
      fromAddress: hit.from_address,
      date: hit.date,
      isRead: hit.is_read,
      location: locationLabel(hit.folder_type, hit.workflow_state),
    };
  }

  async function run(raw: string, id: number) {
    if (!raw) {
      results.value = [];
      endLoading();
      errorText.value = '';
      warnings.value = [];
      return;
    }

    loading.value = true;
    errorText.value = '';

    try {
      let rows: SearchResult[];
      let notes: string[] = [];
      if (backendCommandsAvailable()) {
        const response = await backendSearchMessageHits(raw, requestOptions());
        if (id !== requestId) return;
        cursor = response.next_cursor ?? null;
        hasMore.value = response.has_more ?? false;
        rows = response.hits.map(toResult);
        notes = response.warnings.map(warningText);
      } else {
        rows = [];
      }

      // A slower earlier request must never overwrite a newer result set.
      if (id !== requestId) return;
      results.value = rows;
      warnings.value = notes;
    } catch (err) {
      if (id !== requestId) return;
      results.value = [];
      warnings.value = [];
      errorText.value = err instanceof Error ? err.message : t('search.failed');
    } finally {
      if (id === requestId) {
        endLoading();
      }
    }
  }

  function clearPending() {
    pendingResults.value = null;
    pendingCount.value = 0;
  }

  /**
   * Re-run the active query in the background and park the outcome.
   *
   * Deliberately does not touch `results`, `loading` or `errorText`: a search
   * that is on screen must not flicker, reorder or report an error because a
   * newsletter happened to arrive. A failure here is silent — the visible
   * result set is still valid, just not the newest.
   */
  async function runLive(id: number) {
    const raw = trimmedQuery.value;
    if (!raw || !backendCommandsAvailable()) return;

    try {
      const response = await backendSearchMessageHits(raw, requestOptions());
      // The user typed, switched account or refreshed while this was in flight.
      if (id !== requestId) return;

      pendingCursor = response.next_cursor ?? null;
      pendingHasMore = response.has_more ?? false;
      const rows = response.hits.map(toResult);
      const shown = new Set(results.value.map((row) => row.messageId));
      const added = rows.filter((row) => !shown.has(row.messageId)).length;
      if (added === 0) {
        // Nothing new for this query — drop any stale badge rather than
        // leaving a count that no longer matches the parked set.
        clearPending();
        return;
      }
      pendingResults.value = rows;
      pendingCount.value = added;
    } catch {
      // Keep showing what we have; the next event or keystroke retries.
    }
  }

  /** Promote the parked result set into the visible one. */
  function refresh() {
    if (pendingResults.value) {
      requestId += 1;
      results.value = pendingResults.value;
      cursor = pendingCursor;
      hasMore.value = pendingHasMore;
      loadingMore.value = false;
    }
    clearPending();
  }

  async function loadMore() {
    if (!hasMore.value || !cursor || loading.value || loadingMore.value) return;
    const id = requestId;
    const nextCursor = cursor;
    loadingMore.value = true;
    errorText.value = '';
    try {
      const response = await backendSearchMessageHits(trimmedQuery.value, {
        ...requestOptions(), cursor: nextCursor,
      });
      if (id !== requestId) return;
      const seen = new Set(results.value.map((row) => row.threadId));
      results.value = [...results.value, ...response.hits.map(toResult).filter((row) => {
        if (seen.has(row.threadId)) return false;
        seen.add(row.threadId);
        return true;
      })];
      cursor = response.next_cursor ?? null;
      hasMore.value = response.has_more && cursor !== null;
    } catch (error) {
      if (id === requestId) errorText.value = error instanceof Error ? error.message : t('search.failed');
    } finally {
      if (id === requestId) loadingMore.value = false;
    }
  }

  if (options?.liveUpdates) {
    useMailEvents({
      onIncoming: () => {
        if (!trimmedQuery.value) return;
        // A sync ingesting a batch emits one event per message; without this
        // the composable would fire a query per mail.
        if (liveHandle) clearTimeout(liveHandle);
        liveHandle = setTimeout(() => void scheduleSearch(trimmedQuery.value, requestId, true), SEARCH_DEBOUNCE_MS);
      },
    });
  }

  watch(
    [
      trimmedQuery,
      () => accountsStore.activeAccountId,
      () => accountFilter?.value,
      () => accountsStore.visibilityKey,
      // By value: a recomputed scope naming the same list is the same search.
      () => JSON.stringify(options?.listScope?.value ?? null),
    ],
    ([next]) => {
      if (debounceHandle) clearTimeout(debounceHandle);
      if (liveHandle) clearTimeout(liveHandle);
      // Results parked for the previous query say nothing about this one.
      clearPending();
      queuedSearch = undefined;
      requestId += 1;
      cursor = null;
      hasMore.value = false;
      loadingMore.value = false;
      const id = requestId;

      if (!next) {
        results.value = [];
        endLoading();
        errorText.value = '';
        warnings.value = [];
        return;
      }

      beginLoading();
      debounceHandle = setTimeout(() => void scheduleSearch(next, id), SEARCH_DEBOUNCE_MS);
    },
  );

  // Guarded so the composable is also usable outside of a component setup().
  if (getCurrentInstance()) {
    onBeforeUnmount(() => {
      if (debounceHandle) clearTimeout(debounceHandle);
      if (liveHandle) clearTimeout(liveHandle);
      if (loadingIndicatorHandle) clearTimeout(loadingIndicatorHandle);
      queuedSearch = undefined;
      requestId += 1;
    });
  }

  return {
    query,
    results,
    loading,
    loadingMore,
    hasMore,
    loadMore,
    showLoading,
    errorText,
    warnings,
    hasQuery,
    pendingCount,
    refresh,
  };
}
