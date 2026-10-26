// @vitest-environment jsdom
import {
  computed,
  defineComponent,
  nextTick,
  reactive,
  ref,
  type ComputedRef,
  type Ref,
} from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SearchResult } from '@/composables/useSearch';

type SearchState = {
  query: Ref<string>;
  results: Ref<SearchResult[]>;
  loading: Ref<boolean>;
  showLoading: Ref<boolean>;
  errorText: Ref<string>;
  warnings: Ref<string[]>;
  hasQuery: ComputedRef<boolean>;
  pendingCount: Ref<number>;
  refresh: ReturnType<typeof vi.fn>;
  hasMore: Ref<boolean>;
  loadingMore: Ref<boolean>;
  loadMore: ReturnType<typeof vi.fn>;
};

const harness = vi.hoisted(() => ({
  route: undefined as {
    name: string;
    query: Record<string, string>;
  } | undefined,
  router: { replace: vi.fn() },
  searchState: undefined as SearchState | undefined,
  serverState: undefined as {
    serverHits: Ref<unknown[]>;
    serverLoading: Ref<boolean>;
    serverError: Ref<string>;
    serverQueried: Ref<boolean>;
    fetchingHitId: Ref<string>;
    canSearchServer: Ref<boolean>;
    searchServer: ReturnType<typeof vi.fn>;
    openServerHit: ReturnType<typeof vi.fn>;
  } | undefined,
  localSearchOptions: undefined as {
    accountFilter: Ref<string>;
    liveUpdates: boolean;
  } | undefined,
  serverSearchOptions: undefined as { accountFilter: Ref<string> } | undefined,
  accounts: undefined as {
    accounts: Array<{ id: string; email: string }>;
    activeAccountId: string;
  } | undefined,
  threads: {
    ensureThreadLoaded: vi.fn(),
    selectThread: vi.fn(),
  },
  pushRecent: vi.fn(),
  ui: { isSinglePane: false, registerBackNavigation: vi.fn((_entry: { isOpen: () => boolean; dismiss: () => void }) => vi.fn()) },
}));

vi.mock('@/stores/ui', () => ({ useUiStore: () => harness.ui }));
vi.mock('vue-router', () => ({
  useRoute: () => harness.route,
  useRouter: () => harness.router,
}));
vi.mock('@/composables/useSearch', () => ({
  SEARCH_DEBOUNCE_MS: 120,
  useSearch: (options: { accountFilter: Ref<string>; liveUpdates: boolean }) => {
    harness.localSearchOptions = options;
    return harness.searchState!;
  },
}));
vi.mock('@/composables/useServerSearch', () => ({
  useServerSearch: (options: { accountFilter: Ref<string> }) => {
    harness.serverSearchOptions = options;
    return harness.serverState!;
  },
}));
vi.mock('@/composables/useRecentSearches', () => ({
  useRecentSearches: () => ({ push: harness.pushRecent }),
}));
vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => harness.accounts,
}));
vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => harness.threads,
}));

import SearchView from './SearchView.vue';

const AppButtonStub = defineComponent({
  inheritAttrs: false,
  props: { disabled: Boolean },
  template: '<button v-bind="$attrs" :disabled="disabled"><slot /></button>',
});

const ListDetailLayoutStub = defineComponent({
  template: '<div><slot name="list" /><slot name="detail" /></div>',
});

const ThreadViewStub = defineComponent({
  props: { threadId: { type: String, required: true }, showBack: Boolean },
  template: '<div class="thread-view-stub" :data-thread-id="threadId" />',
});

const EmptyStateStub = defineComponent({
  template: '<div class="empty-state-stub"><slot /></div>',
});

function result(messageId: string, threadId: string): SearchResult {
  return {
    messageId,
    threadId,
    subject: `Subject ${threadId}`,
    snippet: `Snippet ${threadId}`,
    snippetSegments: [{ text: `Snippet ${threadId}`, marked: false }],
    fromName: 'Sender',
    fromAddress: 'sender@example.com',
    date: '2026-08-30T10:00:00Z',
    isRead: true,
    location: '',
  };
}

function makeSearchState(initialQuery = ''): SearchState {
  const query = ref(initialQuery);
  return {
    query,
    results: ref([]),
    loading: ref(false),
    showLoading: ref(false),
    errorText: ref(''),
    warnings: ref([]),
    hasQuery: computed(() => query.value.trim().length > 0),
    pendingCount: ref(0),
    refresh: vi.fn(),
    hasMore: ref(false),
    loadingMore: ref(false),
    loadMore: vi.fn(),
  };
}

function mountView(query: Record<string, string> = { q: 'invoice' }) {
  harness.route = reactive({ name: 'search', query });
  harness.router.replace.mockReset();
  harness.searchState = makeSearchState();
  harness.serverState = {
    serverHits: ref([]),
    serverLoading: ref(false),
    serverError: ref(''),
    serverQueried: ref(false),
    fetchingHitId: ref(''),
    canSearchServer: ref(false),
    searchServer: vi.fn(),
    openServerHit: vi.fn(),
  };
  harness.localSearchOptions = undefined;
  harness.serverSearchOptions = undefined;
  harness.accounts = reactive({
    accounts: [{ id: 'account-1', email: 'one@example.com' }],
    activeAccountId: 'account-1',
  });
  harness.threads.ensureThreadLoaded.mockReset().mockResolvedValue(undefined);
  harness.threads.selectThread.mockReset();
  harness.pushRecent.mockReset();

  return mount(SearchView, {
    global: {
      stubs: {
        AppButton: AppButtonStub,
        Avatar: true,
        DrawerButton: true,
        EmptyState: EmptyStateStub,
        ListDetailLayout: ListDetailLayoutStub,
        ThreadView: ThreadViewStub,
        TimeAgo: true,
      },
    },
  });
}

afterEach(() => {
  vi.useRealTimers();
});

beforeEach(() => {
  vi.clearAllMocks();
  harness.ui.isSinglePane = false;
  harness.router.replace.mockReset();
  harness.threads.ensureThreadLoaded.mockResolvedValue(undefined);
});

describe('SearchView', () => {
  it('hydrates from the route, follows route changes, updates the URL, and clears account scope', async () => {
    const wrapper = mountView({ q: 'initial' });
    await nextTick();

    expect(wrapper.get('.search-input').element).toHaveProperty('value', 'initial');
    expect(harness.localSearchOptions?.liveUpdates).toBe(true);
    expect(harness.localSearchOptions?.accountFilter).toBe(
      harness.serverSearchOptions?.accountFilter,
    );
    expect(harness.localSearchOptions?.accountFilter.value).toBe('account-1');
    expect(wrapper.get('.scope-chip').text()).toContain('one@example.com');

    await wrapper.get('.scope-chip__clear').trigger('click');
    expect(harness.localSearchOptions?.accountFilter.value).toBe('');
    expect(wrapper.find('.scope-chip').exists()).toBe(false);

    harness.route!.query = { q: 'contact', global: '1' };
    await nextTick();
    expect(harness.searchState?.query.value).toBe('contact');
    expect(harness.localSearchOptions?.accountFilter.value).toBe('');

    vi.useFakeTimers();
    await wrapper.get('.search-input').setValue('new query');
    vi.advanceTimersByTime(120);
    expect(harness.router.replace).toHaveBeenLastCalledWith({
      name: 'search',
      query: { q: 'new query', global: '1' },
    });
  });

  it('writes the URL once per typing burst, and at once on Enter', async () => {
    const wrapper = mountView({ q: '' });
    await nextTick();
    vi.useFakeTimers();
    harness.router.replace.mockReset();

    const input = wrapper.get('.search-input');
    for (const value of ['i', 'in', 'inv', 'invo']) {
      await input.setValue(value);
      vi.advanceTimersByTime(50);
    }
    expect(harness.router.replace).not.toHaveBeenCalled();
    vi.advanceTimersByTime(120);
    expect(harness.router.replace).toHaveBeenCalledTimes(1);
    expect(harness.router.replace).toHaveBeenLastCalledWith({
      name: 'search',
      query: { q: 'invo', global: undefined },
    });

    await input.setValue('invoice');
    await input.trigger('keydown', { key: 'Enter' });
    expect(harness.router.replace).toHaveBeenCalledTimes(2);
    expect(harness.router.replace).toHaveBeenLastCalledWith({
      name: 'search',
      query: { q: 'invoice', global: undefined },
    });
    vi.advanceTimersByTime(500);
    expect(harness.router.replace).toHaveBeenCalledTimes(2);
  });

  it('drops a pending URL write once the route has moved on', async () => {
    const wrapper = mountView({ q: '' });
    await nextTick();
    vi.useFakeTimers();
    harness.router.replace.mockReset();

    await wrapper.get('.search-input').setValue('invoice');
    harness.route!.name = 'inbox';
    wrapper.unmount();
    vi.advanceTimersByTime(500);

    expect(harness.router.replace).not.toHaveBeenCalled();
  });

  it('shows chips before a query, then collapses them behind an accessible disclosure', async () => {
    const wrapper = mountView({ q: '' });
    await nextTick();
    expect(wrapper.findAll('.operator-chip')).toHaveLength(9);
    await wrapper.get('.search-input').setValue('plan');
    expect(wrapper.findAll('.operator-chip')).toHaveLength(0);
    expect(wrapper.get('.search-tips').attributes('aria-expanded')).toBe('false');
    await wrapper.get('.search-tips').trigger('click');
    expect(wrapper.findAll('.operator-chip')).toHaveLength(9);
    expect(wrapper.get('.search-tips').attributes('aria-expanded')).toBe('true');
    wrapper.unmount();
  });

  it('highlights query terms in subjects and snippets without interpreting mail as HTML', async () => {
    const wrapper = mountView({ q: 'plan' });
    await nextTick();
    harness.searchState!.results.value = [{
      ...result('message-1', 'thread-1'),
      subject: '<img src=x onerror=alert(1)> PLAN',
      snippet: 'A plan & <script> for today',
      snippetSegments: [{ text: 'A plan & <script> for today', marked: false }],
    }];
    await nextTick();
    expect(wrapper.get('.result-subject mark').text()).toBe('PLAN');
    expect(wrapper.get('.result-snippet mark').text()).toBe('plan');
    expect(wrapper.get('.result-subject').text()).toContain('<img src=x');
    expect(wrapper.find('.result-subject img').exists()).toBe(false);
    expect(wrapper.find('.result-snippet script').exists()).toBe(false);
    wrapper.unmount();
  });

  it('clears the query with the named clear control and restores the operator chips', async () => {
    const wrapper = mountView({ q: 'plan' });
    await nextTick();
    await wrapper.get('.search-clear').trigger('click');
    expect(harness.searchState!.query.value).toBe('');
    expect(wrapper.find('.search-clear').exists()).toBe(false);
    expect(wrapper.findAll('.operator-chip')).toHaveLength(9);
    wrapper.unmount();
  });

  it('appends an operator chip to the current query', async () => {
    const wrapper = mountView({ q: 'from:alice@example.com' });
    await nextTick();

    await wrapper.get('.search-tips').trigger('click');
    const chip = wrapper.findAll('.operator-chip')
      .find((button) => button.text() === 'has:attachment');
    expect(chip).toBeDefined();
    await chip!.trigger('click');

    expect(harness.searchState?.query.value).toBe('from:alice@example.com has:attachment');
    expect(harness.router.replace).toHaveBeenLastCalledWith({
      name: 'search',
      query: { q: 'from:alice@example.com has:attachment' },
    });
  });

  it('shows delayed loading and selects the first result once hits arrive', async () => {
    const wrapper = mountView();
    const first = result('message-1', 'thread-1');
    harness.searchState!.showLoading.value = true;
    await nextTick();
    expect(wrapper.find('.search-skeleton').exists()).toBe(true);

    harness.searchState!.showLoading.value = false;
    harness.searchState!.results.value = [first];
    await nextTick();
    await flushPromises();

    expect(harness.threads.ensureThreadLoaded).toHaveBeenCalledWith('thread-1');
    expect(wrapper.find('.result-row--active').exists()).toBe(true);
    expect(wrapper.get('.thread-view-stub').attributes('data-thread-id')).toBe('thread-1');
  });

  it('loads and selects the thread belonging to the clicked hit', async () => {
    const wrapper = mountView();
    harness.searchState!.results.value = [
      result('message-1', 'thread-1'),
      result('message-2', 'thread-2'),
    ];
    await nextTick();
    harness.threads.ensureThreadLoaded.mockClear();

    await wrapper.findAll('.result-row')[1]!.trigger('click');
    await flushPromises();

    expect(harness.pushRecent).toHaveBeenCalledWith('invoice');
    expect(harness.threads.ensureThreadLoaded).toHaveBeenCalledWith('thread-2');
    expect(harness.threads.selectThread).toHaveBeenCalledWith('thread-2');
    expect(wrapper.get('.thread-view-stub').attributes('data-thread-id')).toBe('thread-2');
  });

  it('refreshes parked hits when the pending-results action is clicked', async () => {
    const wrapper = mountView();
    harness.searchState!.pendingCount.value = 2;
    await nextTick();

    const pending = wrapper.get('.pending-hits');
    expect(pending.text()).toContain('2');
    await pending.trigger('click');
    expect(harness.searchState!.refresh).toHaveBeenCalledOnce();
  });
  it('keeps the results visible on mobile until a hit opens and supports both back paths', async () => {
    harness.ui.isSinglePane = true;
    const wrapper = mountView({ q: 'Google', global: '1' });
    harness.searchState!.results.value = [result('message-1', 'thread-1')];
    await nextTick();
    expect(wrapper.find('.thread-view-stub').exists()).toBe(false);
    await wrapper.get('.result-row').trigger('click');
    expect(wrapper.find('.thread-view-stub').exists()).toBe(true);
    expect(wrapper.findComponent(ThreadViewStub).props('showBack')).toBe(true);
    wrapper.findComponent(ThreadViewStub).vm.$emit('back');
    await nextTick();
    expect(wrapper.find('.thread-view-stub').exists()).toBe(false);
    expect(harness.searchState!.query.value).toBe('Google');
    expect(harness.searchState!.results.value).toHaveLength(1);
    await wrapper.get('.result-row').trigger('click');
    const navigation = harness.ui.registerBackNavigation.mock.calls.at(-1)?.[0] as unknown as { isOpen: () => boolean; dismiss: () => void };
    expect(navigation.isOpen()).toBe(true);
    navigation.dismiss();
    await nextTick();
    expect(wrapper.find('.thread-view-stub').exists()).toBe(false);
  });

  it('labels partial results and requests another page', async () => {
    const wrapper = mountView();
    harness.searchState!.results.value = [result('message-1', 'thread-1')];
    harness.searchState!.hasMore.value = true;
    await nextTick();
    expect(wrapper.get('.result-count').text()).toContain('loaded');
    const button = wrapper.findAll('button').find((item) => item.text() === 'Load more results');
    await button!.trigger('click');
    expect(harness.searchState!.loadMore).toHaveBeenCalledOnce();
  });

  it('restores the active account when navigating from global search to a scoped destination', async () => {
    const wrapper = mountView({ q: 'contact', global: '1' });
    await nextTick();
    expect(harness.localSearchOptions?.accountFilter.value).toBe('');
    harness.route!.query = { q: 'invoice' };
    await nextTick();
    expect(harness.localSearchOptions?.accountFilter.value).toBe('account-1');
    expect(wrapper.get('.scope-chip').text()).toContain('one@example.com');
    expect(harness.searchState!.query.value).toBe('invoice');
  });

});
