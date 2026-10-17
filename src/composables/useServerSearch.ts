import { computed, ref, watch, type Ref } from 'vue';
import {
  backendFetchServerSearchHit,
  backendServerSearch,
  backendCommandsAvailable,
} from '@/services/backend';
import type { BackendServerSearchHit } from '@/types/messages';
import { useAccountsStore } from '@/stores/accounts';
import { useThreadsStore } from '@/stores/threads';
import { useRecentSearches } from '@/composables/useRecentSearches';
import { useToast } from '@/composables/useToast';
import { transportErrorCause } from '@/utils/transportLabels';
import { t } from '@/i18n/runtime';

export interface ServerHitEntry {
  accountId: string;
  hit: BackendServerSearchHit;
}

/**
 * Search provider-backed mail that has not been ingested locally yet.
 *
 * The interface deliberately exposes only view state and two user actions.
 * Provider filtering, account scoping, request invalidation, parallel account
 * searches, and the ingest-to-thread handoff all live behind this seam so a
 * search view cannot accidentally diverge from another server-search caller.
 */
export function useServerSearch(options: {
  query: Ref<string>;
  accountFilter: Ref<string>;
  onThreadSelected: (threadId: string) => void;
}) {
  const accounts = useAccountsStore();
  const threads = useThreadsStore();
  const toast = useToast();
  const { push: pushRecent } = useRecentSearches();

  const serverHits = ref<ServerHitEntry[]>([]);
  const serverLoading = ref(false);
  const serverError = ref('');
  const serverQueried = ref(false);
  /** remote_id of the hit currently being downloaded. */
  const fetchingHitId = ref('');

  /** Accounts whose provider supports a server-side query (Gmail/Outlook). */
  const serverAccounts = computed(() =>
    accounts.accounts.filter(
      (account) =>
        (account.provider === 'gmail' || account.provider === 'outlook')
        && (!options.accountFilter.value || account.id === options.accountFilter.value),
    ),
  );

  const serverAccountScope = computed(() =>
    JSON.stringify(
      serverAccounts.value.map((account) => [account.id, account.provider]),
    ),
  );

  const canSearchServer = computed(
    () => backendCommandsAvailable() && serverAccounts.value.length > 0,
  );

  // Server results belong to the query and account scope they were fetched
  // for. Invalidate in-flight work as well, so a slow old response cannot
  // repopulate a fresh query or clear a newer fetch guard.
  let requestId = 0;
  function reset() {
    requestId += 1;
    serverHits.value = [];
    serverError.value = '';
    serverQueried.value = false;
    serverLoading.value = false;
    fetchingHitId.value = '';
  }

  watch([options.query, options.accountFilter, serverAccountScope], reset, { flush: 'sync' });

  async function searchServer() {
    const raw = options.query.value.trim();
    if (!raw || serverLoading.value || !canSearchServer.value) return;

    const id = ++requestId;
    const accountScope = serverAccountScope.value;
    const isCurrentRequest = () =>
      id === requestId && accountScope === serverAccountScope.value;
    serverLoading.value = true;
    serverError.value = '';
    try {
      const responses = await Promise.all(
        serverAccounts.value.map(async (account) => {
          const hits = await backendServerSearch({ accountId: account.id, query: raw });
          return hits.map((hit) => ({ accountId: account.id, hit }));
        }),
      );
      if (!isCurrentRequest()) return;
      serverHits.value = responses.flat();
      serverQueried.value = true;
    } catch (error) {
      // Offline, an expired token and a rate limit each need a different
      // answer from the reader; one sentence for all three needs none.
      const cause = transportErrorCause(error);
      if (isCurrentRequest()) {
        serverError.value = cause
          ? t('search.server.failedCause', { cause })
          : t('search.server.failed');
      }
    } finally {
      if (isCurrentRequest()) serverLoading.value = false;
    }
  }

  /** Download a remote hit through the normal ingest pipeline and show it. */
  async function openServerHit(entry: ServerHitEntry) {
    if (fetchingHitId.value) return;

    const id = requestId;
    fetchingHitId.value = entry.hit.remote_id;
    try {
      const fetched = await backendFetchServerSearchHit({
        accountId: entry.accountId,
        remoteId: entry.hit.remote_id,
      });
      if (id !== requestId) return;

      pushRecent(options.query.value);
      serverHits.value = serverHits.value.filter(
        (candidate) => candidate.hit.remote_id !== entry.hit.remote_id,
      );
      await threads.ensureThreadLoaded(fetched.thread_id);
      if (id !== requestId) return;
      threads.selectThread(fetched.thread_id);
      options.onThreadSelected(fetched.thread_id);
    } catch (error) {
      const cause = transportErrorCause(error);
      if (id === requestId) {
        toast.error(cause
          ? t('search.server.fetchFailedCause', { cause })
          : t('search.server.fetchFailed'));
      }
    } finally {
      if (id === requestId) fetchingHitId.value = '';
    }
  }

  return {
    serverHits,
    serverLoading,
    serverError,
    serverQueried,
    fetchingHitId,
    canSearchServer,
    searchServer,
    openServerHit,
  };
}
