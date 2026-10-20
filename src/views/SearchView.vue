<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import DrawerButton from '@/components/layout/DrawerButton.vue';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { SEARCH_DEBOUNCE_MS, useSearch, type SearchResult } from '@/composables/useSearch';
import { useServerSearch } from '@/composables/useServerSearch';
import { useRecentSearches } from '@/composables/useRecentSearches';
import { useThreadsStore } from '@/stores/threads';
import { useUiStore } from '@/stores/ui';
import { useAccountsStore } from '@/stores/accounts';
import Avatar from '@/components/common/Avatar.vue';
import TimeAgo from '@/components/common/TimeAgo.vue';
import AppButton from '@/components/ui/AppButton.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import ListDetailLayout from '@/components/layout/ListDetailLayout.vue';
import ThreadView from '@/components/mail/ThreadView.vue';
import { useI18n } from 'vue-i18n';

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const operatorHints = computed(() => [
  'from:name@example.com',
  'to:name@example.com',
  t('search.hints.subject'),
  t('search.hints.tag'),
  'in:sent',
  'has:attachment',
  'is:unread',
  'after:7d',
  '-newsletter',
]);
const ui = useUiStore();
const threads = useThreadsStore();
const accounts = useAccountsStore();
/**
 * The scope is a visible, removable filter chip: opening the search from a
 * scoped context carries the account in, deleting the chip searches all
 * accounts. '' = global.
 */
const accountFilterId = ref('');
const { query, results, loading, showLoading, errorText, warnings, hasQuery, pendingCount, refresh, hasMore, loadingMore, loadMore }
  = useSearch({
    accountFilter: accountFilterId,
    liveUpdates: true,
  });
const { push: pushRecent } = useRecentSearches();

const accountFilterEmail = computed(
  () => accounts.accounts.find((account) => account.id === accountFilterId.value)?.email ?? '',
);

/** The result whose conversation shows in the detail pane. */
const selectedThreadId = ref('');

const {
  serverHits,
  serverLoading,
  serverError,
  serverQueried,
  fetchingHitId,
  canSearchServer,
  searchServer,
  openServerHit,
} = useServerSearch({
  query,
  accountFilter: accountFilterId,
  onThreadSelected: (threadId) => { selectedThreadId.value = threadId; },
});

onMounted(() => {
  const initial = route.query.q;
  if (typeof initial === 'string') {
    query.value = initial;
  }
  // Entering from a scoped context carries the account in as a chip —
  // unless the caller (sidebar contact click) explicitly wants global.
  if (route.query.global !== '1' && accounts.activeAccountId) {
    accountFilterId.value = accounts.activeAccountId;
  }
});

// Clicking another contact in the sidebar pushes a new ?q= while this view
// stays mounted — adopt it instead of ignoring everything after mount.
watch(
  () => [route.query.q, route.query.global] as const,
  ([next, global], [, previousGlobal]) => {
    if (route.name !== 'search') return;
    if (typeof next === 'string' && next !== query.value) {
      query.value = next;
    }
    if (global === '1') {
      accountFilterId.value = '';
    } else if (previousGlobal === '1') {
      // A new scoped destination from the omni panel replaces a global search.
      accountFilterId.value = accounts.activeAccountId || '';
    }
  },
);

// Keep the URL shareable without spamming history entries. Debounced like
// the search itself: WebKit throttles `history.replaceState` and throws once
// a fast typist exceeds its budget.
let urlSyncHandle: ReturnType<typeof setTimeout> | undefined;
function cancelUrlSync() {
  if (urlSyncHandle === undefined) return;
  clearTimeout(urlSyncHandle);
  urlSyncHandle = undefined;
}
function syncQueryToUrl() {
  cancelUrlSync();
  // A timer outliving a navigation must not pull the user back here.
  if (route.name !== 'search') return;
  void router.replace({ name: 'search', query: { ...route.query, q: query.value || undefined } });
}
function flushUrlSync() {
  if (urlSyncHandle !== undefined) syncQueryToUrl();
}

// Synchronous, so a flush right after an assignment already sees the timer.
watch(query, () => {
  if (ui.isSinglePane) selectedThreadId.value = '';
  cancelUrlSync();
  urlSyncHandle = setTimeout(syncQueryToUrl, SEARCH_DEBOUNCE_MS);
}, { flush: 'sync' });
onBeforeUnmount(flushUrlSync);

// The detail pane follows the results: first hit selected automatically,
// stale selections dropped — like the inbox list. Hits usually point at
// threads outside the loaded page, so fetch the row before showing it.
watch(results, (rows) => {
  if (!rows.some((row) => row.threadId === selectedThreadId.value)) {
    const first = ui.isSinglePane ? '' : rows[0]?.threadId ?? '';
    if (first) void threads.ensureThreadLoaded(first);
    selectedThreadId.value = first;
  }
});

function closeDetail() {
  selectedThreadId.value = '';
}

onBeforeUnmount(ui.registerBackNavigation({
  isOpen: () => ui.isSinglePane && Boolean(selectedThreadId.value),
  dismiss: closeDetail,
}));

watch(accountFilterId, (id) => {
  // Carries the current query, so a pending query sync has nothing left to do.
  cancelUrlSync();
  void router.replace({ name: 'search', query: { ...route.query, q: query.value || undefined, global: id ? undefined : '1' } });
});

/** Selecting a hit shows the conversation in the detail pane, like the inbox. */
function open(result: SearchResult) {
  flushUrlSync();
  // Feeds the omni panel's "Letzte Suchen" group.
  pushRecent(query.value);
  void threads.ensureThreadLoaded(result.threadId);
  threads.selectThread(result.threadId);
  selectedThreadId.value = result.threadId;
}

function appendOperator(token: string) {
  query.value = query.value ? `${query.value.trimEnd()} ${token}` : token;
  // A chip is one deliberate edit, not typing to coalesce.
  flushUrlSync();
}
</script>

<template>
  <ListDetailLayout :has-detail="!!selectedThreadId">
    <template #list>
    <section class="search-pane">
      <header class="search-header" :class="{ 'search-header--loading': showLoading }">
        <DrawerButton /><h2 class="search-title">{{ $t('search.title') }}</h2>
        <input
          v-model="query"
          type="search"
          class="search-input"
          :placeholder="$t('search.placeholder')"
          :aria-label="$t('search.queryLabel')"
          @keydown.enter="flushUrlSync"
        />
        <div class="operator-hints">
          <AppButton
            v-for="token in operatorHints"
            :key="token"
            class="operator-chip"
            @click="appendOperator(token)"
            variant="ghost"
            size="sm"
          >
            {{ token }}
          </AppButton>
        </div>
        <div v-if="accountFilterId" class="scope-row">
          <span class="scope-chip">
            {{ $t('search.accountScope', { account: accountFilterEmail || accountFilterId }) }}
            <IconButton
              class="scope-chip__clear"
              :title="$t('search.searchAllAccounts')"
              :aria-label="$t('search.removeAccountFilter')"
              @click="accountFilterId = ''"
            >×</IconButton>
          </span>
        </div>
      </header>

      <div class="search-scroll">
        <p v-for="warning in warnings" :key="warning" class="warning" role="status">{{ warning }}</p>

        <!-- Mail that arrived after this search ran. Held back so the list does
             not resort while the user is reading a hit. Sits outside the
             state chain below because it is also the only hint that a search
             which found nothing would now find something. -->
        <AppButton
          v-if="pendingCount > 0"
          class="pending-hits"
          variant="soft"
          size="sm"
          @click="refresh"
        >
          {{ $t('search.pendingHits', pendingCount) }}
        </AppButton>

        <p v-if="errorText && results.length === 0" class="state state--error" role="alert">{{ errorText }}</p>

        <!-- Keep the previous result geometry while a new FTS query runs.
             Replacing it with a centered spinner made every keystroke move the
             list and detail pane. The header progress line communicates work
             without taking layout space. -->
        <template v-else-if="results.length > 0">
          <p class="result-count" role="status">{{ $t(hasMore ? 'search.loadedHitCount' : 'search.hitCount', results.length) }}</p>
          <ul class="results" :aria-busy="loading">
            <li v-for="result in results" :key="result.messageId">
              <CardButton
                class="result-row"
                :class="{ 'result-row--active': result.threadId === selectedThreadId }"
                @click="open(result)"
              >
                <Avatar :name="result.fromName" :email="result.fromAddress" :size="34" />
                <span class="result-body">
                  <span class="result-top">
                    <strong :class="{ unread: !result.isRead }">{{ result.fromName }}</strong>
                    <span v-if="result.location" class="result-location">{{ result.location }}</span>
                    <TimeAgo :date="result.date" class="result-time" />
                  </span>
                  <span class="result-subject">{{ result.subject }}</span>
                  <!-- snippet(-1) may echo the subject when the match sits there -->
                  <span v-if="result.snippet !== result.subject" class="result-snippet">
                    <template v-for="(segment, index) in result.snippetSegments" :key="index">
                      <mark v-if="segment.marked">{{ segment.text }}</mark>
                      <template v-else>{{ segment.text }}</template>
                    </template>
                  </span>
                </span>
              </CardButton>
            </li>
          </ul>
          <p v-if="errorText" class="state state--error" role="alert">{{ errorText }}</p>
          <AppButton v-if="hasMore" variant="soft" :disabled="loading || loadingMore" @click="loadMore">
            {{ $t(loadingMore ? 'search.loadingMore' : 'search.loadMore') }}
          </AppButton>
        </template>

        <div v-else-if="showLoading" class="search-skeleton" role="status" :aria-label="$t('search.running')">
          <div v-for="index in 5" :key="index" class="search-skeleton__row">
            <span class="search-skeleton__avatar" />
            <span class="search-skeleton__body"><i /><i /><i /></span>
          </div>
        </div>
        <div v-else-if="loading" class="search-waiting" aria-hidden="true" />
        <EmptyState
          v-else-if="hasQuery"
          :title="$t('search.noResults.title')"
          :description="$t('search.noResults.description')"
        >
          <template #icon>
            <svg width="40" height="40" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" opacity="0.55">
              <circle cx="7" cy="7" r="5" />
              <path d="M11 11L14 14" />
            </svg>
          </template>
        </EmptyState>
        <EmptyState
          v-else
          :title="$t('search.intro.title')"
          :description="$t('search.intro.description')"
        >
          <template #icon>
            <svg width="40" height="40" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" opacity="0.55">
              <circle cx="7" cy="7" r="5" />
              <path d="M11 11L14 14" />
            </svg>
          </template>
        </EmptyState>

        <template v-if="hasQuery && canSearchServer && !loading">
          <div class="server-search">
            <AppButton
              variant="soft"
              size="sm"
              :disabled="serverLoading"
              @click="searchServer"
            >
              {{ serverLoading ? $t('search.server.running') : $t('search.server.action') }}
            </AppButton>
            <span class="server-hint">{{ $t('search.server.hint') }}</span>
          </div>

          <p v-if="serverError" class="state state--error" role="alert">{{ serverError }}</p>

          <template v-else-if="serverHits.length > 0">
            <p class="result-count" role="status">{{ $t('search.server.hitCount', serverHits.length) }}</p>
            <ul class="results" :aria-label="$t('search.server.results')">
              <li v-for="entry in serverHits" :key="entry.hit.remote_id">
                <CardButton
                  class="result-row"
                  :disabled="Boolean(fetchingHitId)"
                  @click="openServerHit(entry)"
                >
                  <Avatar :name="entry.hit.from" :email="entry.hit.from" :size="34" />
                  <span class="result-body">
                    <span class="result-top">
                      <strong>{{ entry.hit.from }}</strong>
                      <TimeAgo v-if="entry.hit.date" :date="entry.hit.date" class="result-time" />
                    </span>
                    <span class="result-subject">{{ entry.hit.subject }}</span>
                    <span v-if="entry.hit.snippet" class="result-snippet">{{ entry.hit.snippet }}</span>
                    <span class="result-server-note">
                      {{ fetchingHitId === entry.hit.remote_id
                        ? $t('search.server.fetching')
                        : $t('search.server.note') }}
                    </span>
                  </span>
                </CardButton>
              </li>
            </ul>
          </template>

          <p v-else-if="serverQueried && !serverLoading" class="state">
            {{ $t('search.server.noMore') }}
          </p>
        </template>
      </div>
    </section>
    </template>

    <template #detail>
      <ThreadView v-if="selectedThreadId" :thread-id="selectedThreadId" :show-back="ui.isSinglePane" @back="closeDetail" />
      <div v-else class="detail-placeholder">
        <EmptyState
          :title="$t('search.detail.title')"
          :description="$t('search.detail.description')"
        >
          <template #icon>
            <svg width="40" height="40" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" opacity="0.55">
              <circle cx="7" cy="7" r="5" />
              <path d="M11 11L14 14" />
            </svg>
          </template>
        </EmptyState>
      </div>
    </template>
  </ListDetailLayout>
</template>

<style scoped>
.search-pane {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  min-height: 0;
  min-width: 0;
  height: 100%;
}

.search-header {
  position: relative;
  display: grid;
  gap: 8px;
  padding: 14px 14px 10px;
  border-bottom: 1px solid var(--border-color-muted);
}

.search-header::after {
  position: absolute;
  right: 0;
  bottom: -1px;
  left: 0;
  height: 2px;
  content: '';
  background: var(--accent-primary);
  opacity: 0;
  transform: scaleX(0.2);
  transform-origin: left;
  transition: opacity var(--duration-fast) var(--ease-out);
}

.search-header--loading::after {
  opacity: 0.75;
  animation: search-progress 900ms var(--ease-in-out) infinite alternate;
}

@keyframes search-progress {
  to { transform: scaleX(1); }
}

.search-title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  letter-spacing: -0.01em;
  color: var(--text-primary);
}

.scope-row {
  display: flex;
}

/* The account scope as a removable filter chip — visible, never silent. */
.scope-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 6px 2px 9px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-primary) 12%, transparent);
  color: var(--accent-primary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
}

.scope-chip__clear {
  border: 0;
  background: transparent;
  color: inherit;
  font-size: var(--font-size-sm);
  line-height: 1;
  padding: 0 3px;
  cursor: pointer;
  opacity: 0.7;
}

.scope-chip__clear:hover {
  opacity: 1;
}

.search-input {
  width: 100%;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-md);
  padding: 9px 12px;
}

.search-input:focus {
  border-color: var(--accent-primary);
}

.operator-hints {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}

.search-scroll {
  min-height: 0;
  overflow-y: auto;
  scrollbar-gutter: stable;
  padding: 8px 8px 20px;
}

.search-waiting {
  min-height: 360px;
}

.search-skeleton {
  display: grid;
  gap: 2px;
  padding: 4px 6px;
  animation: search-skeleton-pulse 1.2s ease-in-out infinite alternate;
}

.search-skeleton__row {
  display: grid;
  grid-template-columns: 34px minmax(0, 1fr);
  align-items: center;
  gap: 10px;
  min-height: 76px;
  padding: 9px 8px;
}

.search-skeleton__avatar,
.search-skeleton__body i {
  background: var(--bg-raised);
}

.search-skeleton__avatar {
  width: 34px;
  height: 34px;
  border-radius: 50%;
}

.search-skeleton__body {
  display: grid;
  gap: 7px;
}

.search-skeleton__body i {
  display: block;
  width: 42%;
  height: 8px;
  border-radius: var(--radius-full);
}

.search-skeleton__body i:nth-child(2) { width: 72%; }
.search-skeleton__body i:nth-child(3) { width: 58%; opacity: 0.7; }

@keyframes search-skeleton-pulse {
  to { opacity: 0.52; }
}

@media (prefers-reduced-motion: reduce) {
  .search-header--loading::after,
  .search-skeleton {
    animation: none;
  }
}

.warning {
  margin: 4px 6px;
  font-size: var(--font-size-xs);
  color: var(--accent-amber);
}

.state {
  margin: 12px 6px;
  font-size: var(--font-size-sm);
  color: var(--text-tertiary);
}

.state--error {
  color: var(--accent-red);
}

.result-count {
  margin: 4px 6px 6px;
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.pending-hits {
  margin: 0 6px 8px;
}

.results {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 2px;
}

.result-row {
  width: 100%;
  display: flex;
  align-items: flex-start;
  gap: 10px;
  border: 0;
  border-radius: var(--radius-md);
  background: transparent;
  font: inherit;
  text-align: left;
  padding: 9px 8px;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.result-row:hover {
  background: var(--bg-hover);
}

.result-row--active {
  background: color-mix(in srgb, var(--accent-primary) 12%, transparent);
}

.result-body {
  min-width: 0;
  flex: 1;
  display: grid;
  gap: 2px;
}

.result-top {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.result-top strong {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.result-top strong.unread {
  font-weight: var(--weight-bold);
}

.result-time {
  flex-shrink: 0;
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

/* Where the hit lives — folder or workflow state, nothing for plain inbox. */
.result-location {
  flex-shrink: 0;
  padding: 1px 7px;
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
}

.result-subject {
  font-size: var(--font-size-sm);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.result-snippet {
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.result-snippet mark {
  background: color-mix(in srgb, var(--accent-amber) 30%, transparent);
  color: var(--text-primary);
  border-radius: 2px;
  padding: 0 1px;
}

.result-server-note {
  font-size: var(--font-size-xs);
  color: var(--accent-primary);
}

.server-search {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 14px 6px 6px;
}

.server-hint {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.detail-placeholder {
  height: 100%;
  display: grid;
  place-items: center;
}
</style>
