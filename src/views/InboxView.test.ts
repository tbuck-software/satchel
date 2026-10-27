// @vitest-environment jsdom

import { defineComponent, reactive } from 'vue';
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { answerConfirms, type DialogStub } from '@/test/dialogs';
import type { ConfirmRequest } from '@/composables/useConfirm';

type ThreadSummary = {
  id: string;
  accountId: string;
  workflowState: string;
  category?: string;
};

const harness = vi.hoisted(() => ({
  push: vi.fn(),
  route: { name: 'inbox' },
  blockSender: vi.fn(),
  toastError: vi.fn(),
  selectFromInteraction: vi.fn(),
  selectThread: vi.fn(),
  closeTopicDetail: vi.fn(),
  followUpsLoad: vi.fn(),
  loadMore: vi.fn(),
  registerBackNavigation: vi.fn(() => vi.fn()),
  registerKeyboardNavigation: vi.fn(() => vi.fn()),
  setActiveAccount: vi.fn(),
  backendErrorFor: vi.fn((_source: string) => null as string | null),
  threads: null as Record<string, unknown> | null,
  accounts: null as Record<string, unknown> | null,
  settings: null as Record<string, unknown> | null,
  ui: null as Record<string, unknown> | null,
  topics: null as Record<string, unknown> | null,
  listSearch: null as Record<string, unknown> | null,
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: harness.push }),
  useRoute: () => harness.route,
}));

vi.mock('vue-i18n', () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => harness.threads,
}));

vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => harness.accounts,
}));

vi.mock('@/stores/connection', () => ({
  useConnectionStore: () => ({ backendErrorFor: harness.backendErrorFor }),
}));

vi.mock('@/stores/settings', () => ({
  useSettingsStore: () => harness.settings,
}));

vi.mock('@/stores/ui', () => ({
  THREAD_LIST_WIDTH_LIMITS: { min: 320, max: 760 },
  useUiStore: () => harness.ui,
}));

vi.mock('@/stores/listSearch', () => ({
  useListSearchStore: () => harness.listSearch,
}));

// The real results load the FTS client; the view only places them.
vi.mock('@/components/mail/ListSearchResults.vue', async () => {
  const { defineComponent: define } = await import('vue');
  return {
    __esModule: true,
    default: define({
      name: 'ListSearchResults',
      props: { mailboxLabel: String, openThreadId: String },
      template: '<section data-test="search-results" :data-mailbox="mailboxLabel" :data-open="openThreadId" />',
    }),
  };
});

vi.mock('@/stores/topics', () => ({
  useTopicsStore: () => harness.topics,
}));

vi.mock('@/stores/followUps', () => ({
  useFollowUpsStore: () => ({ load: harness.followUpsLoad }),
}));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({
    archive: vi.fn(),
    blockSender: harness.blockSender,
    snoozeAt: vi.fn(),
    transitionThreads: vi.fn(),
  }),
}));

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ error: harness.toastError, success: vi.fn() }),
}));

vi.mock('@/composables/useListKeyboardNavigation', () => ({
  registerListKeyboardNavigation: harness.registerKeyboardNavigation,
}));

import InboxView from './InboxView.vue';
import ListFilterMenu from '@/components/mail/ListFilterMenu.vue';

enableAutoUnmount(afterEach);

const ThreadGroupListStub = defineComponent({
  name: 'ThreadGroupList',
  props: {
    items: { type: Array, required: true },
    loading: Boolean,
    error: { type: String, required: false, default: null },
    activeThreadId: { type: String, required: false },
    selectedThreadIds: { type: Array, required: true },
  },
  emits: ['activate', 'load-more', 'new-sender-action', 'retry'],
  template: `
    <div
      data-test="thread-list"
      :data-loading="String(loading)"
      :data-error="String(error)"
      :data-active-thread-id="activeThreadId"
      :data-item-count="String(items.length)"
    >
      <slot v-if="!items.length" name="empty" />
      <button data-test="retry" @click="$emit('retry')" />
      <button
        data-test="activate-pointer"
        @click="$emit('activate', { threadId: 'thread-2', shiftKey: false, metaKey: false, ctrlKey: false })"
      />
      <button
        data-test="activate-keyboard"
        @click="$emit('activate', { threadId: 'thread-2', shiftKey: false, metaKey: false, ctrlKey: false, keyboard: true })"
      />
      <button data-test="load-more" @click="$emit('load-more')" />
      <button
        data-test="block-new-sender"
        @click="$emit('new-sender-action', {
          sender: {
            id: 'sender-1',
            senderName: 'Ada Lovelace',
            senderEmail: 'ada@example.org',
            accountId: 'account-1',
            subject: 'A message',
            previewText: 'Preview',
            count: 1,
            unreadCount: 1,
            lastMessageAt: '2026-08-31T10:00:00.000Z',
            lastMessageAtMs: Date.parse('2026-08-31T10:00:00.000Z'),
            threadIds: ['thread-1'],
            topThreadId: 'thread-1',
            spoofingText: null,
          },
          action: 'block',
        })"
      />
    </div>
  `,
});

const ThreadViewStub = defineComponent({
  name: 'ThreadView',
  props: { threadId: { type: String, required: false } },
  template: '<div data-test="thread-detail" :data-thread-id="threadId" />',
});

const PaneHeaderStub = defineComponent({
  name: 'PaneHeader',
  template: '<header><slot name="leading" /><slot name="actions" /></header>',
});

function thread(id: string): ThreadSummary {
  return {
    id,
    accountId: 'account-1',
    workflowState: 'inbox',
    category: 'personal',
  };
}

function setup(options: {
  singlePane?: boolean;
  mobile?: boolean;
  loading?: boolean;
  workflowFilter?: string;
  accounts?: Array<{ id: string; displayName?: string; color?: string }>;
  activeAccountId?: string;
} = {}) {
  const first = thread('thread-1');
  harness.threads = reactive({
    workflowFilter: options.workflowFilter ?? 'all',
    mailboxFilter: { kind: 'section', value: 'inbox' },
    activeContactEmail: '',
    activeThreadId: '',
    scopedVisibleThreads: [first],
    activeSimilarGroup: null,
    activeSmartBucket: null,
    activeContact: null,
    smartListEntries: [],
    listEntries: [{ kind: 'thread', thread: first }],
    smartInboxTotalCount: 0,
    visibleThreads: [first],
    threads: [first],
    categoryFilter: 'all',
    selectedThreadIds: [],
    selectedThreads: [],
    threadLoadInFlight: options.loading ?? false,
    newSenderStack: null,
    selectFromInteraction: harness.selectFromInteraction,
    selectThread: harness.selectThread,
    ensureThreadLoaded: vi.fn(),
    loadMore: harness.loadMore,
    closeSimilarGroup: vi.fn(),
    closeSmartBucket: vi.fn(),
    openSimilarGroup: vi.fn(),
    openSmartBucket: vi.fn(),
    setMailboxFilter: vi.fn(),
    setThreadsRead: vi.fn(),
    setThreadCategory: vi.fn(),
    setSenderCategory: vi.fn(),
    clearSelection: vi.fn(),
    load: vi.fn(),
  });
  harness.accounts = reactive({
    activeAccountId: options.activeAccountId ?? 'account-1',
    accounts: options.accounts ?? [{ id: 'account-1' }],
    accountScopeLabel: 'account@example.org',
    setActiveAccount: harness.setActiveAccount,
  });
  harness.settings = reactive({
    inboxGroupingMode: 'classic',
    senderReviewStartedAt: '2026-08-01T10:00:00.000Z',
    setInboxGroupingMode: vi.fn(),
    acceptSender: vi.fn(),
    acceptDomain: vi.fn(),
    completeSenderReviewSetup: vi.fn(),
    setDismissedNewSender: vi.fn(),
  });
  harness.ui = reactive({
    isSinglePane: options.singlePane ?? false,
    isMobile: options.mobile ?? false,
    threadListWidth: 420,
    setThreadListWidth: vi.fn(),
    openOmni: vi.fn(),
    closeMobileSidebar: vi.fn(),
    registerBackNavigation: harness.registerBackNavigation,
    // Context menus register while open.
    registerOverlay: vi.fn(() => vi.fn()),
  });
  const listSearch = reactive({
    query: '',
    scope: 'mailbox',
    openThreadId: '',
    resultThreadIds: [] as string[],
    get active() {
      return listSearch.query.trim().length > 0;
    },
    // Closures, like the store's: the view passes them on unbound.
    setQuery: (next: string) => { listSearch.query = next; },
    setScope: (next: string) => { listSearch.scope = next; },
    open: vi.fn(),
    clear: () => {
      listSearch.query = '';
      listSearch.scope = 'mailbox';
      listSearch.openThreadId = '';
      listSearch.resultThreadIds = [];
    },
  });
  harness.listSearch = listSearch;
  harness.topics = reactive({
    activeTopics: [],
    finishedTopics: [],
    activeTopicId: null,
    openTopicDetail: vi.fn(),
    closeTopicDetail: harness.closeTopicDetail,
    create: vi.fn(),
  });
}

function render() {
  return mount(InboxView, {
    global: {
      mocks: { $t: (key: string) => key },
      stubs: {
        AppButton: true,
        Avatar: true,
        CountBadge: true,
        IconButton: true,
        PaneHeader: PaneHeaderStub,
        SectionHeading: true,
        SmartBucketIcon: true,
        SnoozePicker: true,
        ThreadGroupList: ThreadGroupListStub,
        ThreadPaneSkeleton: true,
        ThreadView: ThreadViewStub,
        TopicDetail: true,
        TopicsFolderCard: true,
      },
    },
  });
}

describe('InboxView', () => {
  let confirms: DialogStub<ConfirmRequest, boolean>;

  beforeEach(() => {
    harness.push.mockReset().mockResolvedValue(undefined);
    harness.selectFromInteraction.mockReset();
    harness.selectThread.mockReset();
    harness.closeTopicDetail.mockReset();
    harness.followUpsLoad.mockReset().mockResolvedValue(undefined);
    harness.loadMore.mockReset().mockResolvedValue(undefined);
    harness.registerBackNavigation.mockClear();
    harness.registerKeyboardNavigation.mockClear();
    harness.blockSender.mockReset();
    harness.toastError.mockReset();
    harness.backendErrorFor.mockReset().mockReturnValue(null);
    confirms = answerConfirms(true);
  });

  afterEach(() => {
    confirms.stop();
    vi.restoreAllMocks();
  });

  it('uses the first scoped thread for the desktop list and detail fallback', () => {
    setup();

    const wrapper = render();

    expect(wrapper.get('[data-test="thread-list"]').attributes('data-active-thread-id')).toBe('thread-1');
    expect(wrapper.get('[data-test="thread-detail"]').attributes('data-thread-id')).toBe('thread-1');
    expect(harness.push).not.toHaveBeenCalled();
  });

  it('keeps loading and empty list state in the list component', () => {
    setup({ loading: true });
    (harness.threads!.scopedVisibleThreads as ThreadSummary[]) = [];
    (harness.threads!.listEntries as unknown[]) = [];
    (harness.threads!.visibleThreads as ThreadSummary[]) = [];

    const wrapper = render();
    const list = wrapper.get('[data-test="thread-list"]');

    expect(list.attributes('data-loading')).toBe('true');
    expect(list.attributes('data-item-count')).toBe('0');
    expect(list.attributes('data-active-thread-id')).toBeUndefined();
    expect(wrapper.get('[data-test="thread-detail"]').attributes('data-thread-id')).toBeUndefined();
  });

  it('hands the standing thread load failure to the list and retries on the same path', async () => {
    setup();
    harness.backendErrorFor.mockImplementation(
      (source: string) => (source === 'threads' ? 'db is locked' : null),
    );

    const wrapper = render();
    expect(wrapper.get('[data-test="thread-list"]').attributes('data-error')).toBe('db is locked');

    await wrapper.get('[data-test="retry"]').trigger('click');
    expect(harness.threads!.load).toHaveBeenCalledTimes(1);
  });

  it.each(['later', 'aside'])('uses one contextual empty state for %s without the topics card', (workflow) => {
    setup();
    harness.threads!.workflowFilter = workflow;
    harness.threads!.mailboxFilter = { kind: 'all', value: null };
    harness.threads!.listEntries = [];
    harness.threads!.smartListEntries = [];
    harness.topics!.activeTopics = [{ id: 'topic-1', title: 'An unrelated topic' }];
    const wrapper = render();
    expect(wrapper.find('.topics-folder').exists()).toBe(false);
    expect(wrapper.get('.empty h3').text()).toBe(`inbox.empty.${workflow}.title`);
    expect(wrapper.get('.empty p').text()).toBe(`inbox.empty.${workflow}.description`);
    expect(wrapper.findAll('.empty button')).toHaveLength(0);
  });

  it('selects pointer-activated mail in place on desktop', async () => {
    setup();
    const wrapper = render();

    await wrapper.get('[data-test="activate-pointer"]').trigger('click');

    expect(harness.closeTopicDetail).toHaveBeenCalledOnce();
    expect(harness.selectFromInteraction).toHaveBeenCalledWith(
      'thread-2',
      { shiftKey: false, metaKey: false, ctrlKey: false },
      true,
    );
    expect(harness.push).not.toHaveBeenCalled();
  });

  it('opens activated mail as a route in single-pane mode', async () => {
    setup({ singlePane: true });
    const wrapper = render();

    expect(wrapper.find('[data-test="thread-detail"]').exists()).toBe(false);
    expect(wrapper.get('[data-test="thread-list"]').attributes('data-active-thread-id')).toBeUndefined();

    await wrapper.get('[data-test="activate-pointer"]').trigger('click');

    expect(harness.selectFromInteraction).toHaveBeenCalledOnce();
    expect(harness.push).toHaveBeenCalledWith({ name: 'thread', params: { id: 'thread-2' } });
  });

  it('loads follow-ups immediately when Later is open', () => {
    setup({ workflowFilter: 'later' });

    render();

    expect(harness.followUpsLoad).toHaveBeenCalledOnce();
  });

  it('keeps a new-sender suggestion when blocking fails', async () => {
    setup();
    harness.blockSender.mockRejectedValue(new Error('blocked sender persistence failed'));

    const wrapper = render();
    await wrapper.get('[data-test="block-new-sender"]').trigger('click');

    expect(harness.blockSender).toHaveBeenCalledWith({
      pattern: 'ada@example.org',
      accountId: 'account-1',
    });
    expect(harness.settings!.setDismissedNewSender).not.toHaveBeenCalled();
    expect(harness.threads!.load).not.toHaveBeenCalled();
    expect(harness.selectThread).not.toHaveBeenCalled();
    expect(harness.toastError).toHaveBeenCalledWith(
      'Could not block ada@example.org. The sender is still in the review list.',
    );
  });

  it.each(['accept', 'block'] as const)('opens the next new sender in the detail pane after %s', async (action) => {
    setup();
    const sender = {
      senderEmail: 'ada@example.org',
      accountId: 'account-1',
      lastMessageAt: '2026-08-31T10:00:00.000Z',
      threadIds: ['thread-1'],
      topThreadId: 'thread-1',
    };
    const next = {
      ...sender,
      senderEmail: 'next@example.org',
      threadIds: ['thread-2'],
      topThreadId: 'thread-2',
    };
    harness.threads!.newSenderStack = { senders: [sender, next] };
    harness.threads!.activeThreadId = 'thread-1';
    harness.selectThread.mockImplementation((id) => { harness.threads!.activeThreadId = id; });
    const removeSender = () => { harness.threads!.newSenderStack = { senders: [next] }; };
    (harness.settings!.acceptSender as ReturnType<typeof vi.fn>).mockImplementation(removeSender);
    (harness.settings!.setDismissedNewSender as ReturnType<typeof vi.fn>).mockImplementation(removeSender);
    const wrapper = render();

    wrapper.getComponent(ThreadGroupListStub).vm.$emit('new-sender-action', { sender, action });
    await flushPromises();

    expect(wrapper.get('[data-test="thread-detail"]').attributes('data-thread-id')).toBe('thread-2');
  });

  it('steps onto the new-sender banner, expands it with Enter and then walks its rows', async () => {
    setup();
    const first = harness.threads!.listEntries as Array<Record<string, unknown>>;
    const senders = ['s1', 's2', 's3', 's4'].map((id) => ({ id: `new-sender-${id}`, topThreadId: `thread-${id}` }));
    harness.threads!.listEntries = [
      { kind: 'new-sender-stack', id: 'new-sender-stack', stack: { id: 'new-sender-stack', senders } },
      ...first,
    ];
    harness.ui!.newSendersExpanded = false;
    harness.ui!.toggleNewSendersExpanded = vi.fn(() => {
      harness.ui!.newSendersExpanded = !harness.ui!.newSendersExpanded;
    });
    render();
    const keyboard = (harness.registerKeyboardNavigation.mock.calls[0] as unknown as [{
      move: (delta: 1 | -1) => boolean;
      open: () => boolean;
    }])[0];

    // Topics sits above the list in the classic layout; the banner comes next.
    keyboard.move(1);
    expect(harness.selectThread).not.toHaveBeenCalled();
    expect(keyboard.open()).toBe(true);
    expect(harness.ui!.toggleNewSendersExpanded).toHaveBeenCalledTimes(1);
    await flushPromises();

    // Expanded: the same three rows the list shows, each selecting its thread.
    keyboard.move(1);
    keyboard.move(1);
    keyboard.move(1);
    keyboard.move(1);
    expect(harness.selectThread.mock.calls.map(([id]) => id)).toEqual([
      'thread-s1', 'thread-s2', 'thread-s3', 'thread-1',
    ]);
  });

  it('ticks a new sender off only after the block went through', async () => {
    setup();
    harness.blockSender.mockResolvedValue(undefined);

    const wrapper = render();
    await wrapper.get('[data-test="block-new-sender"]').trigger('click');

    const dismiss = harness.settings!.setDismissedNewSender as ReturnType<typeof vi.fn>;
    expect(dismiss).toHaveBeenCalledWith('ada@example.org', '2026-08-31T10:00:00.000Z');
    // The block has to be durable before the sender leaves the review list.
    expect(harness.blockSender.mock.invocationCallOrder[0]).toBeLessThan(
      dismiss.mock.invocationCallOrder[0]!,
    );
    expect(harness.threads!.load).toHaveBeenCalled();
    expect(harness.toastError).not.toHaveBeenCalled();
  });
  /*
   * A phone used to show the accounts twice: as pills above the list and in
   * the filter menu beside the search field. The menu is the one control, on
   * every form factor; the strip keeps only the Inbox categories.
   */
  it('leaves the account choice to the filter menu on a phone', () => {
    setup({
      mobile: true,
      accounts: [
        { id: 'account-1', displayName: 'Work', color: '#166fdd' },
        { id: 'account-2', displayName: 'Uni', color: '#b4485d' },
      ],
    });

    const wrapper = render();
    const labels = wrapper.findAll('.filter-pill').map((pill) => pill.text());
    expect(labels).not.toContain('Work');
    expect(labels).not.toContain('inbox.allAccounts');
    expect(wrapper.find('.list-filter-menu__trigger').exists()).toBe(true);
  });

  it('offers account and layout filtering in one list filter menu', async () => {
    setup({
      accounts: [
        { id: 'account-1', displayName: 'Work' },
        { id: 'account-2', displayName: 'Uni' },
      ],
    });
    expect(render().findAll('.filter-pill').map((pill) => pill.text())).not.toContain('Work');

    const wrapper = mount(ListFilterMenu, {
      props: { showLayout: true },
      global: {
        mocks: { $t: (key: string) => key },
        stubs: { teleport: true },
      },
    });

    await wrapper.get('[aria-haspopup="menu"]').trigger('click');
    // The menu loads on first open.
    await vi.waitFor(() => expect(wrapper.findAll('[role="menuitemcheckbox"]')).toHaveLength(3));
    const items = wrapper.findAll('[role="menuitemcheckbox"]');
    expect(items.map((item) => item.text())).toEqual(['folders.allAccounts', 'Work', 'Uni']);
    expect(items[1]!.attributes('aria-checked')).toBe('true');
    expect(wrapper.findAll('[role="menuitemradio"]').map((item) => item.text()))
      .toEqual(['inbox.viewGrouped', 'inbox.viewList']);
    await items[2]!.trigger('click');
    expect(harness.setActiveAccount).toHaveBeenCalledWith('account-2');
    expect(wrapper.find('[role="menu"]').exists()).toBe(false);

    await wrapper.get('[aria-haspopup="menu"]').trigger('click');
    await wrapper.findAll('[role="menuitemradio"]')[0]!.trigger('click');
    expect(harness.settings!.setInboxGroupingMode).toHaveBeenCalledWith('smart');
  });

  it('hides the filter menu when there is nothing to choose', () => {
    setup({ accounts: [] });
    const wrapper = mount(ListFilterMenu, {
      props: { showLayout: false },
      global: { mocks: { $t: (key: string) => key }, stubs: { teleport: true } },
    });
    expect(wrapper.find('[aria-haspopup="menu"]').exists()).toBe(false);
  });

  it('puts a search field above the list instead of the palette launcher', () => {
    setup();
    const wrapper = render();

    const field = wrapper.get('input[type="search"]');
    expect(field.attributes('aria-label')).toBe('inbox.listSearch.label');
    expect(wrapper.find('.search-field').exists()).toBe(false);
    expect(wrapper.find('[data-test="search-results"]').exists()).toBe(false);
  });

  it('replaces the list with search results and opens a hit in the reader', async () => {
    setup();
    const wrapper = render();

    await wrapper.get('input[type="search"]').setValue('invoice');
    harness.listSearch!.resultThreadIds = ['thread-7', 'thread-8'];
    await flushPromises();

    const results = wrapper.get('[data-test="search-results"]');
    expect(results.attributes('data-mailbox')).toBe('folders.inbox');
    // The first hit stands in until one is opened, like the list's own fallback.
    expect(wrapper.get('[data-test="thread-detail"]').attributes('data-thread-id')).toBe('thread-7');

    harness.listSearch!.openThreadId = 'thread-8';
    await flushPromises();
    expect(wrapper.get('[data-test="thread-detail"]').attributes('data-thread-id')).toBe('thread-8');
    // The mailbox list stays mounted underneath, so its scroll survives.
    expect(wrapper.find('[data-test="thread-list"]').exists()).toBe(true);
  });

  it('gives the previous thread back when Escape clears the search', async () => {
    setup();
    harness.threads!.activeThreadId = 'thread-1';
    const wrapper = render();
    const field = wrapper.get('input[type="search"]');

    await field.setValue('invoice');
    harness.listSearch!.resultThreadIds = ['thread-7'];
    await flushPromises();
    harness.selectThread.mockClear();

    await field.trigger('keydown', { key: 'Escape' });
    await flushPromises();

    expect(harness.listSearch!.query).toBe('');
    expect(wrapper.find('[data-test="search-results"]').exists()).toBe(false);
    expect(harness.selectThread).toHaveBeenCalledWith('thread-1');
    expect(wrapper.get('[data-test="thread-detail"]').attributes('data-thread-id')).toBe('thread-1');
  });

  it('ends the search when another mailbox is picked', async () => {
    setup();
    const wrapper = render();
    await wrapper.get('input[type="search"]').setValue('invoice');
    expect(harness.listSearch!.query).toBe('invoice');

    harness.threads!.mailboxFilter = { kind: 'section', value: 'sent' };
    await flushPromises();
    expect(harness.listSearch!.query).toBe('');
  });
});
