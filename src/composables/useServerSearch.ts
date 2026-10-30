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

/** Stable identity for provider IDs, which are only unique within an account. */
export function serverHitKey(entry: ServerHitEntry): string {
  return JSON.stringify([entry.accountId, entry.hit.remote_id]);
}

/**
 * Search provider-backed mail missing from the local query results.
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
  /** Account-scoped key of the hit currently being downloaded. */
  const fetchingHitId = ref('');

  /** Accounts whose provider supports a server-side query (Gmail/Outlook). */
  const serverAccounts = computed(() =>
    accounts.accounts.filter(
      (account) =>
        (account.provider === 'gmail' || account.provider === 'outlook')
        && (options.accountFilter.value ? account.id === options.accountFilter.value : accounts.isVisible(account.id)),
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
  let failedAccountIds = new Set<string>();
  function reset() {
    requestId += 1;
    failedAccountIds.clear();
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
    fetchingHitId.value = '';
    const accountScope = serverAccountScope.value;
    const isCurrentRequest = () =>
      id === requestId && accountScope === serverAccountScope.value;
    const requestedAccounts = serverAccounts.value.filter(account =>
      failedAccountIds.size === 0 || failedAccountIds.has(account.id),
    );
    serverLoading.value = true;
    serverError.value = '';
    try {
      const responses = await Promise.allSettled(
        requestedAccounts.map(account => backendServerSearch({ accountId: account.id, query: raw })),
      );
      if (!isCurrentRequest()) return;
      const failures: Array<{ accountId: string; reason: unknown }> = [];
      responses.forEach((response, index) => {
        const account = requestedAccounts[index]!;
        if (response.status === 'fulfilled') {
          serverHits.value = serverHits.value.filter(entry => entry.accountId !== account.id)
            .concat(response.value.map(hit => ({ accountId: account.id, hit })));
          serverQueried.value = true;
        } else {
          failures.push({ accountId: account.id, reason: response.reason });
        }
      });
      failedAccountIds = new Set(failures.map(failure => failure.accountId));
      if (failures.length) {
        const cause = transportErrorCause(failures[0]!.reason);
        const accountNames = requestedAccounts.filter(account => failedAccountIds.has(account.id))
          .map(account => account.email || account.id).join(', ');
        serverError.value = serverQueried.value
          ? (cause ? t('search.server.partialFailedCause', { accounts: accountNames, cause }) : t('search.server.partialFailed', { accounts: accountNames }))
          : (cause ? t('search.server.failedCause', { cause }) : t('search.server.failed'));
      }
    } finally {
      if (isCurrentRequest()) serverLoading.value = false;
    }
  }

  /** Download a remote hit through the normal ingest pipeline and show it. */
  async function openServerHit(entry: ServerHitEntry) {
    if (fetchingHitId.value) return;

    const id = requestId;
    fetchingHitId.value = serverHitKey(entry);
    try {
      const fetched = await backendFetchServerSearchHit({
        accountId: entry.accountId,
        remoteId: entry.hit.remote_id,
      });
      if (id !== requestId) return;

      pushRecent(options.query.value);
      serverHits.value = serverHits.value.filter(
        (candidate) => serverHitKey(candidate) !== serverHitKey(entry),
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
