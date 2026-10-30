// @vitest-environment jsdom

import { defineComponent, reactive } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendDraft } from '@/types/composer';
import type { Thread } from '@/types/mail';

interface DraftsHarness {
  items: BackendDraft[];
  loading: boolean;
  load: ReturnType<typeof vi.fn>;
  readonly sorted: BackendDraft[];
  readonly scheduled: BackendDraft[];
}

interface ComposerHarness {
  openNew: ReturnType<typeof vi.fn>;
  openDraft: ReturnType<typeof vi.fn>;
}

interface ThreadsHarness {
  threads: Thread[];
  ensureThreadLoaded: ReturnType<typeof vi.fn>;
}

interface DraftActionsHarness {
  isBusy: ReturnType<typeof vi.fn>;
  discard: ReturnType<typeof vi.fn>;
}

interface UiHarness {
  isSinglePane: boolean;
  registerBackNavigation: ReturnType<typeof vi.fn<(entry: { isOpen: () => boolean; dismiss: () => void }) => () => void>>;
}

const harness = vi.hoisted(() => ({
  ui: null as UiHarness | null,
  drafts: null as DraftsHarness | null,
  composer: null as ComposerHarness | null,
  accounts: null as { accounts: unknown[] } | null,
  threads: null as ThreadsHarness | null,
  draftActions: null as DraftActionsHarness | null,
  items: null as BackendDraft[] | null,
  busyIds: null as Set<string> | null,
  connection: null as { errors: Record<string, string>; backendErrorFor: (source: string) => string | null } | null,
}));

vi.mock('vue-i18n', () => ({
  useI18n: () => ({
    locale: { value: 'en-US' },
    t: (key: string) => key,
  }),
}));

vi.mock('@/stores/ui', () => ({ useUiStore: () => harness.ui }));

vi.mock('@/stores/drafts', () => ({
  useDraftsStore: () => harness.drafts,
}));

vi.mock('@/stores/composer', () => ({
  useComposerStore: () => harness.composer,
}));

vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => harness.accounts,
}));

vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => harness.threads,
}));

vi.mock('@/stores/connection', () => ({
  useConnectionStore: () => harness.connection,
}));

vi.mock('@/composables/useDraftActions', () => ({
  useDraftActions: () => harness.draftActions,
}));

import DraftsView from './DraftsView.vue';

const ListDetailLayoutStub = defineComponent({
  name: 'ListDetailLayout',
  props: { hasDetail: Boolean },
  template: `
    <div data-test="layout" :data-has-detail="String(hasDetail)">
      <div data-test="list"><slot name="list" /></div>
      <div data-test="detail"><slot name="detail" /></div>
    </div>
  `,
});

const PaneHeaderStub = defineComponent({
  name: 'PaneHeader',
  props: {
    title: { type: String, required: true },
    subtitle: { type: String, required: true },
    count: { type: Number, required: true },
  },
  template: `
    <header data-test="pane-header" :data-title="title" :data-subtitle="subtitle" :data-count="String(count)">
      <slot name="actions" />
    </header>
  `,
});

const AppButtonStub = defineComponent({
  name: 'AppButton',
  inheritAttrs: false,
  props: { disabled: Boolean },
  emits: ['click'],
  template: '<button data-test="app-button" v-bind="$attrs" :disabled="disabled" @click="$emit(\'click\')"><slot /></button>',
});

const IconButtonStub = defineComponent({
  name: 'IconButton',
  inheritAttrs: false,
  props: {
    disabled: Boolean,
    title: { type: String, required: true },
  },
  emits: ['click'],
  template: '<button data-test="icon-button" :title="title" :disabled="disabled" @click="$emit(\'click\')"><slot /></button>',
});

const EmptyStateStub = defineComponent({
  name: 'EmptyState',
  props: {
    title: { type: String, required: true },
    description: { type: String, required: true },
    actionLabel: { type: String, required: false },
  },
  emits: ['action'],
  template: `
    <div data-test="empty-state">
      <span data-test="empty-title">{{ title }}</span>
      <span data-test="empty-description">{{ description }}</span>
      <button v-if="actionLabel" data-test="empty-action" type="button" @click="$emit('action')">{{ actionLabel }}</button>
    </div>
  `,
});

const SectionHeadingStub = defineComponent({
  name: 'SectionHeading',
  props: {
    label: { type: String, required: true },
    count: { type: Number, required: true },
  },
  template: '<h2 data-test="section-heading" :data-count="String(count)">{{ label }}</h2>',
});

const ThreadViewStub = defineComponent({
  name: 'ThreadView',
  props: {
    threadId: { type: String, required: true },
    collapseToLatest: Boolean,
    showBack: Boolean,
  },
  template: '<div data-test="thread-view" :data-thread-id="threadId" />',
});

const TimeAgoStub = defineComponent({
  name: 'TimeAgo',
  props: { date: { type: String, required: true } },
  template: '<time data-test="time-ago">{{ date }}</time>',
});

const AvatarStub = defineComponent({
  name: 'Avatar',
  props: {
    name: { type: String, required: true },
    email: { type: String, required: false },
  },
  template: '<span data-test="avatar" :data-name="name" :data-email="email" />',
});

const CountBadgeStub = defineComponent({
  name: 'CountBadge',
  props: { value: { type: Number, required: true } },
  template: '<span data-test="count-badge">{{ value }}</span>',
});

const SkeletonRowStub = defineComponent({
  name: 'SkeletonRow',
  template: '<div data-test="skeleton-row" />',
});

function makeDraft(id: string, overrides: Partial<BackendDraft> = {}): BackendDraft {
  return {
    id,
    account_id: 'account-1',
    thread_id: null,
    to: ['recipient@example.com'],
    cc: [],
    bcc: [],
    subject: `Subject ${id}`,
    body: `Body ${id}`,
    body_html: null,
    scheduled_for: null,
    attachments: [],
    updated_at: new Date(Date.now() - 60_000).toISOString(),
    ...overrides,
  };
}

function makeThread(id: string, overrides: Partial<Thread> = {}): Thread {
  return {
    id,
    accountId: 'account-1',
    subject: `Thread ${id}`,
    participants: [{ name: 'Recipient', email: 'recipient@example.com' }],
    snippet: 'Thread snippet',
    lastMessageAt: new Date().toISOString(),
    lastMessageAtMs: Date.parse(new Date().toISOString()),
    messageCount: 2,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['Inbox'],
    tags: [],
    ...overrides,
  };
}

function setup(items: BackendDraft[] = []) {
  harness.ui = reactive({
    isSinglePane: false,
    registerBackNavigation: vi.fn((_entry: { isOpen: () => boolean; dismiss: () => void }) => vi.fn()),
  });
  harness.items = reactive(items) as BackendDraft[];
  harness.busyIds = reactive(new Set<string>()) as Set<string>;
  harness.drafts = reactive({
    items: harness.items,
    loading: false,
    load: vi.fn().mockResolvedValue(undefined),
    get sorted() {
      return [...harness.items!].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
    },
    get scheduled() {
      return harness.items!.filter(
        (draft: BackendDraft) => draft.scheduled_for && new Date(draft.scheduled_for) > new Date(),
      );
    },
  });
  harness.composer = {
    openNew: vi.fn(),
    openDraft: vi.fn(),
  };
  harness.accounts = reactive({ accounts: [] });
  harness.threads = reactive({
    threads: [] as Thread[],
    ensureThreadLoaded: vi.fn().mockResolvedValue(undefined),
  });
  harness.draftActions = {
    isBusy: vi.fn((id: string) => harness.busyIds!.has(id)),
    discard: vi.fn().mockResolvedValue(true),
  };
  const errors = reactive<Record<string, string>>({});
  harness.connection = {
    errors,
    backendErrorFor: (source: string) => errors[source] ?? null,
  };
}

function render() {
  return mount(DraftsView, {
    global: {
      mocks: { $t: (key: string) => key },
      stubs: {
        AppButton: AppButtonStub,
        Avatar: AvatarStub,
        CountBadge: CountBadgeStub,
        EmptyState: EmptyStateStub,
        IconButton: IconButtonStub,
        ListDetailLayout: ListDetailLayoutStub,
        PaneHeader: PaneHeaderStub,
        SectionHeading: SectionHeadingStub,
        SkeletonRow: SkeletonRowStub,
        ThreadView: ThreadViewStub,
        TimeAgo: TimeAgoStub,
      },
    },
  });
}

describe('DraftsView', () => {
  beforeEach(() => {
    setup();
  });

  it('starts single-pane navigation at the list and returns through Back or system Back', async () => {
    setup([makeDraft('one'), makeDraft('two')]);
    harness.ui!.isSinglePane = true;
    const wrapper = render();
    const layout = wrapper.getComponent(ListDetailLayoutStub);
    const navigation = harness.ui!.registerBackNavigation.mock.calls[0]![0];
    expect(layout.props('hasDetail')).toBe(false);
    await wrapper.findAll('.row__open')[0]!.trigger('click');
    expect(layout.props('hasDetail')).toBe(true);
    expect(navigation.isOpen()).toBe(true);
    const selectedSubject = wrapper.get('.preview__subject').text();
    harness.items!.push(makeDraft('newer', { updated_at: new Date(Date.now() + 1000).toISOString() }));
    await flushPromises();
    expect(wrapper.get('.preview__subject').text()).toBe(selectedSubject);
    await wrapper.get('.preview__header [data-test="icon-button"]').trigger('click');
    expect(layout.props('hasDetail')).toBe(false);
    await wrapper.findAll('.row__open')[1]!.trigger('click');
    navigation.dismiss();
    await flushPromises();
    expect(layout.props('hasDetail')).toBe(false);
    expect(navigation.isOpen()).toBe(false);
    const unregister = harness.ui!.registerBackNavigation.mock.results[0]!.value;
    wrapper.unmount();
    expect(unregister).toHaveBeenCalledOnce();
  });

  it('returns from a selected reply conversation to the drafts list', async () => {
    setup([makeDraft('reply', { thread_id: 'thread-one' })]);
    harness.ui!.isSinglePane = true;
    harness.threads!.threads.push(makeThread('thread-one'));
    const wrapper = render();
    await wrapper.get('.row__open').trigger('click');
    const conversation = wrapper.getComponent(ThreadViewStub);
    expect(conversation.props('showBack')).toBe(true);
    conversation.vm.$emit('back');
    await flushPromises();
    expect(wrapper.getComponent(ListDetailLayoutStub).props('hasDetail')).toBe(false);
    wrapper.unmount();
  });

  it('groups future drafts under scheduled and the rest under open', async () => {
    const scheduled = makeDraft('scheduled', {
      subject: 'Send later',
      scheduled_for: new Date(Date.now() + 60 * 60_000).toISOString(),
      updated_at: new Date(Date.now()).toISOString(),
    });
    const open = makeDraft('open', { subject: 'Finish this' });
    const wrapper = render();
    harness.items!.push(scheduled, open);
    await flushPromises();

    const sections = wrapper.findAll('section.group');
    expect(sections).toHaveLength(2);
    expect(sections[0]!.get('[data-test="section-heading"]').text()).toBe('drafts.groups.scheduled');
    expect(sections[1]!.get('[data-test="section-heading"]').text()).toBe('drafts.title');
    expect(sections[0]!.get('.row').text()).toContain('Send later');
    expect(sections[1]!.get('.row').text()).toContain('Finish this');
    expect(wrapper.find('.row--scheduled').exists()).toBe(true);
    expect(harness.drafts!.load).toHaveBeenCalledOnce();
  });

  it('shows the empty state and starts a new message from its action', async () => {
    const wrapper = render();
    await flushPromises();

    expect(wrapper.find('[data-test="empty-state"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="empty-title"]').text()).toBe('drafts.empty.title');

    await wrapper.get('[data-test="empty-action"]').trigger('click');
    expect(harness.composer!.openNew).toHaveBeenCalledOnce();
  });

  it('shows an error with retry instead of the empty state when the list failed to load', async () => {
    harness.connection!.errors.drafts = 'database is locked';
    const wrapper = render();
    await flushPromises();

    expect(wrapper.find('[data-test="list"] [data-test="empty-state"]').exists()).toBe(false);
    const error = wrapper.get('[role="alert"]');
    expect(error.text()).toContain('drafts.loadFailed');
    expect(error.text()).toContain('database is locked');

    harness.drafts!.load.mockClear();
    await error.get('[data-test="app-button"]').trigger('click');
    expect(harness.drafts!.load).toHaveBeenCalledOnce();
  });

  it('selects drafts and shows either a thread preview or the draft preview', async () => {
    const threadDraft = makeDraft('thread-draft', {
      thread_id: 'thread-1',
      subject: 'Conversation draft',
      updated_at: new Date(Date.now()).toISOString(),
    });
    const standalone = makeDraft('standalone', {
      subject: 'Standalone draft',
      body: 'Plain body for preview',
      updated_at: new Date(Date.now() - 60_000).toISOString(),
    });
    harness.threads!.threads.push(makeThread('thread-1'));
    const wrapper = render();
    harness.items!.push(threadDraft, standalone);
    await flushPromises();

    expect(harness.threads!.ensureThreadLoaded).toHaveBeenCalledWith('thread-1');
    expect(wrapper.find('[data-test="thread-view"]').attributes('data-thread-id')).toBe('thread-1');

    await wrapper.findAll('.row__open')[1]!.trigger('click');
    expect(wrapper.find('[data-test="thread-view"]').exists()).toBe(false);
    expect(wrapper.get('.preview__subject').text()).toBe('Standalone draft');
    expect(wrapper.get('.preview__text').text()).toBe('Plain body for preview');
    expect(wrapper.get('.preview__line').text()).toContain('recipient@example.com');
  });

  it('opens the selected draft from both row and preview edit actions', async () => {
    const draft = makeDraft('edit-me');
    const wrapper = render();
    harness.items!.push(draft);
    await flushPromises();

    await wrapper.get('.row__open').trigger('dblclick');
    await wrapper.get('.preview__actions [data-test="app-button"]').trigger('click');

    expect(harness.composer!.openDraft).toHaveBeenNthCalledWith(1, 'edit-me');
    expect(harness.composer!.openDraft).toHaveBeenNthCalledWith(2, 'edit-me');
  });

  it('discards a draft and disables all actions while that draft is busy', async () => {
    const available = makeDraft('available');
    const busy = makeDraft('busy', {
      updated_at: new Date(Date.now()).toISOString(),
    });
    harness.busyIds!.add('busy');
    const wrapper = render();
    harness.items!.push(available, busy);
    await flushPromises();

    const busyRow = wrapper.get('.row--busy');
    expect(busyRow.findAll('[data-test="icon-button"]').every((button) =>
      (button.element as HTMLButtonElement).disabled,
    )).toBe(true);
    expect(busyRow.find('.row__open').exists()).toBe(true);

    const availableRow = wrapper.findAll('.row').find((row) => !row.classes().includes('row--busy'))!;
    await availableRow.findAll('[data-test="icon-button"]')[1]!.trigger('click');
    await flushPromises();
    expect(harness.draftActions!.discard).toHaveBeenCalledWith('available');
  });
  it.each(['imapUidValidityMissing', 'imapUidValidityChanged'])(
    'shows per-item folder recovery for %s while valid drafts stay editable', async (key) => {
      const invalid = makeDraft('unavailable', { thread_id: 'old-thread', subject: '', body: '', to: [],
        content_error: JSON.stringify({ key: `backend.errors.${key}`, params: { folder: 'Work/Drafts' } }),
      });
      harness.threads!.threads.push(makeThread('old-thread'));
      harness.items!.push(invalid, makeDraft('valid'));
      const wrapper = render();
      await flushPromises();
      const row = wrapper.findAll('.row')[0]!;
      expect(row.text()).toContain('Work/Drafts');
      expect(row.text()).toContain('Sync the folder');
      await row.get('.row__open').trigger('dblclick');
      expect(harness.composer!.openDraft).not.toHaveBeenCalled();
      expect((row.findAll('[data-test="icon-button"]')[0]!.element as HTMLButtonElement).disabled).toBe(true);
      expect(wrapper.get('[role="alert"]').text()).toContain('Work/Drafts');
      expect(wrapper.find('[data-test="thread-view"]').exists()).toBe(false);
      await wrapper.findAll('.row')[1]!.get('.row__open').trigger('dblclick');
      expect(harness.composer!.openDraft).toHaveBeenCalledWith('valid');
    },
  );

});
