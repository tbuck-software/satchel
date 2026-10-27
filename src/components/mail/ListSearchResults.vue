<script setup lang="ts">
/*
 * The search results that replace the thread list while the list search has
 * a query. Loaded on the first keystroke: the FTS client, live updates and
 * thread resolution are not startup code.
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import ThreadGroupList from '@/components/mail/ThreadGroupList.vue';
import { useListSearch, type ListSearchMailbox } from '@/composables/useListSearch';
import { registerListKeyboardNavigation } from '@/composables/useListKeyboardNavigation';
import { useListSearchStore, type ListSearchScope } from '@/stores/listSearch';
import { useThreadsStore } from '@/stores/threads';
import { useUiStore } from '@/stores/ui';

const props = defineProps<{
  /** Name of the list being searched, for the scope switch. */
  mailboxLabel: string;
  mailbox: ListSearchMailbox;
  /** The thread the reading pane shows; drawn as the open row. */
  openThreadId?: string;
}>();

const { t } = useI18n();
const router = useRouter();
const store = useListSearchStore();
const threads = useThreadsStore();
const ui = useUiStore();

const search = useListSearch({ mailbox: () => props.mailbox });

watch(
  search.resultThreads,
  (rows) => {
    store.resultThreadIds = rows.map((thread) => thread.id);
    // Checking rows, ranges and select-all run over the results while they
    // cover the list; the selection bar and its actions stay the Inbox's.
    threads.setSelectionList(rows);
  },
  { immediate: true },
);
onBeforeUnmount(() => threads.setSelectionList(null));

const scopeOptions = computed(() => [
  { value: 'mailbox', label: props.mailboxLabel },
  { value: 'all', label: t('folders.allMail') },
]);

const countLabel = computed(() => {
  const count = search.resultThreads.value.length;
  if (search.loading.value) return '';
  return search.hasMore.value
    ? t('inbox.listSearch.loadedResultCount', count)
    : t('inbox.listSearch.resultCount', count);
});

const keyboardFocusKey = ref('');
const keyboardNavigationActive = ref(false);

function show(threadId: string) {
  store.openThreadId = threadId;
  void threads.ensureThreadLoaded(threadId, { retainAcrossListLoads: true });
  // Keyboard actions (`e`, `#`, …) act on the store's active thread.
  threads.selectThread(threadId);
}

interface RowPointer {
  threadId: string;
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}

function modifiers({ shiftKey, metaKey, ctrlKey }: RowPointer) {
  return { shiftKey, metaKey, ctrlKey };
}

function onActivate(payload: RowPointer & { keyboard?: boolean }) {
  keyboardFocusKey.value = `thread:${payload.threadId}`;
  keyboardNavigationActive.value = payload.keyboard ?? false;
  // As in the list: a plain click opens and clears the selection, a
  // modified one extends it.
  threads.selectFromInteraction(payload.threadId, modifiers(payload), true);
  show(payload.threadId);
  if (ui.isSinglePane || payload.keyboard) {
    void router.push({ name: 'thread', params: { id: payload.threadId } });
  }
}

/** The avatar checks a row, as in the list, instead of opening it. */
function onCheck(payload: RowPointer) {
  threads.selectFromInteraction(payload.threadId, modifiers(payload), false);
}

function move(delta: 1 | -1): boolean {
  const ids = store.resultThreadIds;
  if (ids.length === 0) return false;
  const currentKey = keyboardNavigationActive.value ? keyboardFocusKey.value : `thread:${props.openThreadId ?? ''}`;
  const index = ids.findIndex((id) => `thread:${id}` === currentKey);
  // Coming down from the search field lands on the hit already open, not
  // the one after it.
  const fromField = document.activeElement?.closest('.list-search-bar') != null;
  const step = fromField && !keyboardNavigationActive.value ? 0 : delta;
  const nextId = ids[index < 0 ? 0 : Math.min(ids.length - 1, Math.max(0, index + step))];
  if (!nextId) return false;
  keyboardFocusKey.value = `thread:${nextId}`;
  keyboardNavigationActive.value = true;
  if (!ui.isSinglePane) show(nextId);
  return true;
}

function open(): boolean {
  const id = keyboardFocusKey.value.replace(/^thread:/, '') || props.openThreadId || store.resultThreadIds[0];
  if (!id) return false;
  onActivate({ threadId: id, keyboard: true, shiftKey: false, metaKey: false, ctrlKey: false });
  return true;
}

/** Esc from the results leaves the search, back to the list it narrowed. */
function back(): boolean {
  store.clear();
  return true;
}

onBeforeUnmount(registerListKeyboardNavigation({ move, open, back }));

function dismissKeyboardNavigation() {
  keyboardNavigationActive.value = false;
}

function openAdvancedSearch() {
  void router.push({
    name: 'search',
    query: { q: store.query.trim(), global: store.scope === 'all' ? '1' : undefined },
  });
}
</script>

<template>
  <section
    class="list-search-results"
    :class="{ 'list-search-results--busy': search.showProgress.value }"
    :aria-label="$t('inbox.listSearch.results')"
    @pointerdown="dismissKeyboardNavigation"
  >
    <div class="list-search-results__bar">
      <SegmentedControl
        :model-value="store.scope"
        :options="scopeOptions"
        :aria-label="$t('inbox.listSearch.scope')"
        @update:model-value="store.setScope($event as ListSearchScope)"
      />
      <span class="list-search-results__count" role="status">{{ countLabel }}</span>
      <AppButton variant="ghost" size="sm" :title="$t('inbox.listSearch.advancedTitle')" @click="openAdvancedSearch">
        {{ $t('inbox.listSearch.advanced') }}
      </AppButton>
    </div>

    <p v-for="warning in search.warnings.value" :key="warning" class="list-search-results__note" role="status">{{ warning }}</p>
    <p v-if="search.errorText.value" class="list-search-results__note list-search-results__note--error" role="alert">
      {{ search.errorText.value }}
    </p>
    <AppButton
      v-if="search.pendingCount.value > 0"
      class="list-search-results__pending"
      variant="soft"
      size="sm"
      @click="search.refresh()"
    >
      {{ $t('search.pendingHits', search.pendingCount.value) }}
    </AppButton>

    <div class="list-search-results__list">
      <ThreadGroupList
        :items="search.entries.value"
        :loading="search.loading.value"
        :pagination-busy="search.loadingMore.value"
        :active-thread-id="ui.isSinglePane ? undefined : props.openThreadId"
        :focused-target-key="keyboardFocusKey"
        :keyboard-navigation-active="keyboardNavigationActive"
        :selected-thread-ids="threads.selectedThreadIds"
        :sender-review-setup-complete="true"
        :bulk-busy="false"
        @activate="onActivate"
        @check="onCheck"
        @load-more="search.loadMore()"
      >
        <template #empty>
          <EmptyState
            :title="$t('search.noResults.title')"
            :description="search.narrowed.value
              ? $t('inbox.listSearch.noResultsIn', { mailbox: props.mailboxLabel })
              : $t('search.noResults.description')"
            :action-label="search.narrowed.value ? $t('inbox.listSearch.searchAllMail') : undefined"
            @action="store.setScope('all')"
          >
            <template #icon>
              <AppIcon size="lg"><circle cx="7" cy="7" r="5" /><path d="M11 11L14 14" /></AppIcon>
            </template>
          </EmptyState>
        </template>
      </ThreadGroupList>
    </div>
  </section>
</template>

<style scoped>
.list-search-results {
  position: relative;
  display: grid;
  grid-template-rows: auto auto auto auto minmax(0, 1fr);
  min-height: 0;
  height: 100%;
  background: var(--bg-list);
}

/* The query is running: a hairline of progress instead of moving rows. */
.list-search-results::before {
  position: absolute;
  top: 0;
  right: 0;
  left: 0;
  z-index: 1;
  height: 2px;
  content: '';
  background: var(--accent-primary);
  opacity: 0;
  transform: scaleX(0.2);
  transform-origin: left;
  transition: opacity var(--duration-fast) var(--ease-out);
}

.list-search-results--busy::before {
  opacity: 0.75;
  animation: list-search-progress 900ms var(--ease-in-out) infinite alternate;
}

@keyframes list-search-progress {
  to { transform: scaleX(1); }
}

@media (prefers-reduced-motion: reduce) {
  .list-search-results--busy::before {
    animation: none;
  }
}

.list-search-results__bar {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  min-width: 0;
  padding: var(--space-xs) var(--space-lg) var(--space-sm);
}

.list-search-results__bar :deep(.segmented) {
  min-width: 0;
  flex-shrink: 1;
}

.list-search-results__bar :deep(.segmented__option) {
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.list-search-results__count {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.list-search-results__note {
  margin: 0 var(--space-lg) var(--space-sm);
  color: var(--accent-amber-soft);
  font-size: var(--font-size-xs);
}

.list-search-results__note--error {
  color: var(--accent-red-soft);
}

.list-search-results__pending {
  justify-self: start;
  margin: 0 var(--space-lg) var(--space-sm);
}

.list-search-results__list {
  min-height: 0;
  min-width: 0;
  overflow: hidden;
}
</style>
