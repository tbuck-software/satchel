// @vitest-environment jsdom

import { defineComponent, nextTick, reactive } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendOutboxItem } from '@/types/outbox';

type OutboxHarness = {
  items: BackendOutboxItem[];
  loading: boolean;
  error: string | null;
  load: ReturnType<typeof vi.fn>;
  retry: ReturnType<typeof vi.fn>;
  cancel: ReturnType<typeof vi.fn>;
};

interface UiHarness {
  isSinglePane: boolean;
  registerBackNavigation: ReturnType<typeof vi.fn<(entry: { isOpen: () => boolean; dismiss: () => void }) => () => void>>;
}

const harness = vi.hoisted(() => ({
  ui: null as UiHarness | null,
  outbox: null as OutboxHarness | null,
  composerOpen: vi.fn(),
  toast: {
    success: vi.fn(),
    info: vi.fn(),
    error: vi.fn(),
  },
}));

vi.mock('@/stores/ui', () => ({ useUiStore: () => harness.ui }));

vi.mock('@/stores/outbox', () => ({
  useOutboxStore: () => harness.outbox,
}));

vi.mock('@/stores/composer', () => ({
  useComposerStore: () => ({ open: harness.composerOpen }),
}));

vi.mock('@/composables/useToast', () => ({
  useToast: () => harness.toast,
}));

import OutboxView from './OutboxView.vue';

const ListDetailLayoutStub = defineComponent({
  props: { hasDetail: Boolean },
  template: '<div data-test="layout"><slot name="list" /><slot name="detail" /></div>',
});

const PaneHeaderStub = defineComponent({
  props: {
    title: { type: String, required: true },
    subtitle: { type: String, default: '' },
    count: { type: Number, default: undefined },
  },
  template: `
    <header data-test="pane-header" :data-title="title" :data-subtitle="subtitle" :data-count="count">
      <slot name="actions" />
    </header>
  `,
});

const SectionHeadingStub = defineComponent({
  props: {
    label: { type: String, required: true },
    count: { type: Number, default: undefined },
  },
  template: '<h3 class="section-heading__label">{{ label }} <span>{{ count }}</span></h3>',
});

const AppButtonStub = defineComponent({
  inheritAttrs: false,
  props: {
    disabled: Boolean,
    title: { type: String, default: undefined },
  },
  emits: ['click'],
  template: '<button class="app-button" v-bind="$attrs" :title="title" :disabled="disabled" @click="$emit(\'click\', $event)"><slot /></button>',
});

const IconButtonStub = defineComponent({
  inheritAttrs: false,
  props: {
    disabled: Boolean,
    title: { type: String, default: undefined },
  },
  emits: ['click'],
  template: '<button class="icon-button" v-bind="$attrs" :title="title" :aria-label="title" :disabled="disabled" @click="$emit(\'click\', $event)"><slot /></button>',
});

const ErrorStateStub = defineComponent({
  props: {
    title: { type: String, required: true },
    detail: { type: String, default: undefined },
  },
  emits: ['retry'],
  template: `
    <div data-test="error-state" role="alert">
      <strong>{{ title }}</strong><span>{{ detail }}</span>
      <button data-test="error-retry" @click="$emit('retry')">retry</button>
    </div>
  `,
});

const EmptyStateStub = defineComponent({
  props: {
    title: { type: String, required: true },
    description: { type: String, required: true },
  },
  template: '<div data-test="empty-state"><strong>{{ title }}</strong><span>{{ description }}</span></div>',
});

const SkeletonRowStub = defineComponent({
  props: { count: { type: Number, required: true } },
  template: '<div data-test="skeleton" :data-count="count" />',
});

const AvatarStub = defineComponent({
  props: { name: { type: String, required: true } },
  template: '<span class="avatar" :data-name="name" />',
});

const TimeAgoStub = defineComponent({
  props: {
    date: { type: String, required: true },
    title: { type: String, default: undefined },
  },
  template: '<time class="time-ago" :title="title">{{ date }}</time>',
});

function makeItem(overrides: Partial<BackendOutboxItem> = {}): BackendOutboxItem {
  return {
    id: 'item-1',
    account_id: 'account-1',
    action_type: 'send_message',
    status: 'pending',
    retry_count: 0,
    created_at: '2026-08-30T10:00:00.000Z',
    updated_at: '2026-08-30T10:00:00.000Z',
    last_error: null,
    next_retry_at: null,
    scheduled_for: null,
    subject: 'A subject',
    recipients: ['person@example.com'],
    ...overrides,
  };
}

function render() {
  return mount(OutboxView, {
    global: {
      stubs: {
        AppButton: AppButtonStub,
        Avatar: AvatarStub,
        EmptyState: EmptyStateStub,
        ErrorState: ErrorStateStub,
        IconButton: IconButtonStub,
        ListDetailLayout: ListDetailLayoutStub,
        PaneHeader: PaneHeaderStub,
        SectionHeading: SectionHeadingStub,
        SkeletonRow: SkeletonRowStub,
        TimeAgo: TimeAgoStub,
      },
    },
  });
}

function resetHarness(items: BackendOutboxItem[] = []) {
  harness.ui = reactive({
    isSinglePane: false,
    registerBackNavigation: vi.fn((_entry: { isOpen: () => boolean; dismiss: () => void }) => vi.fn()),
  });
  harness.outbox = reactive({
    items,
    loading: false,
    error: null,
    load: vi.fn(),
    retry: vi.fn().mockResolvedValue(undefined),
    cancel: vi.fn().mockResolvedValue(undefined),
  });
  harness.composerOpen.mockReset();
  harness.toast.success.mockReset();
  harness.toast.info.mockReset();
  harness.toast.error.mockReset();
}

describe('OutboxView', () => {
  beforeEach(() => {
    resetHarness();
  });

  it('starts single-pane navigation at the queue and returns through Back or system Back', async () => {
    resetHarness([makeItem({ id: 'one' }), makeItem({ id: 'two', subject: 'Second item' })]);
    harness.ui!.isSinglePane = true;
    const wrapper = render();
    const layout = wrapper.getComponent(ListDetailLayoutStub);
    const navigation = harness.ui!.registerBackNavigation.mock.calls[0]![0];
    expect(layout.props('hasDetail')).toBe(false);
    await wrapper.findAll('.row')[1]!.trigger('click');
    expect(layout.props('hasDetail')).toBe(true);
    expect(wrapper.get('.detail__subject').text()).toBe('Second item');
    expect(navigation.isOpen()).toBe(true);
    await wrapper.get('.detail__header .icon-button').trigger('click');
    expect(layout.props('hasDetail')).toBe(false);
    await wrapper.findAll('.row')[0]!.trigger('click');
    navigation.dismiss();
    await nextTick();
    expect(layout.props('hasDetail')).toBe(false);
    expect(navigation.isOpen()).toBe(false);
    const unregister = harness.ui!.registerBackNavigation.mock.results[0]!.value;
    wrapper.unmount();
    expect(unregister).toHaveBeenCalledOnce();
  });

  it('groups statuses in attention order and folds unknown statuses into the queue', () => {
    harness.outbox!.items = [
      makeItem({ id: 'pending', status: 'pending', subject: 'Pending' }),
      makeItem({ id: 'failed', status: 'failed', subject: 'Failed' }),
      makeItem({ id: 'delivered', status: 'delivered', subject: 'Delivered' }),
      makeItem({ id: 'unknown', status: 'provider_state', subject: 'Unknown' }),
    ];

    const wrapper = render();

    expect(wrapper.findAll('.group .section-heading__label').map((heading) => heading.text())).toEqual([
      'Sent · local save failed 1',
      'Failed 1',
      'Queued 2',
    ]);
    expect(wrapper.findAll('.row').map((row) => row.find('.row__subject').text())).toEqual([
      'Delivered',
      'Failed',
      'Pending',
      'Unknown',
    ]);
    expect(wrapper.get('[data-test="pane-header"]').attributes('data-subtitle')).toBe('2 need attention');
    expect(wrapper.get('[data-test="pane-header"]').attributes('data-count')).toBe('4');
  });

  it('marks scheduled work overdue and exposes delivery warnings in the selected detail', async () => {
    harness.outbox!.items = [
      makeItem({
        id: 'scheduled',
        status: 'scheduled',
        scheduled_for: '2020-01-01T10:00:00.000Z',
        subject: 'Scheduled mail',
      }),
      makeItem({
        id: 'warning',
        status: 'delivery_unknown',
        subject: 'Uncertain mail',
        submission_started_at: '2026-08-31T08:59:59.000Z',
        updated_at: '2026-08-31T09:00:00.000Z',
      }),
    ];

    const wrapper = render();

    expect(wrapper.findAll('.group .section-heading__label').map((heading) => heading.text())).toContain('Overdue 1');
    const scheduledRow = wrapper.findAll('.row').find(
      (row) => row.get('.row__subject').text() === 'Scheduled mail',
    )!;
    const warningRow = wrapper.findAll('.row').find(
      (row) => row.get('.row__subject').text() === 'Uncertain mail',
    )!;
    expect(scheduledRow.get('.row__time').text()).toBe('Overdue');
    expect(warningRow.find('.row__marker svg').exists()).toBe(true);

    await warningRow.trigger('click');

    expect(wrapper.get('.detail__status').text()).toBe('Delivery status uncertain');
    expect(wrapper.get('.detail__subject').text()).toBe('Uncertain mail');
    expect(wrapper.get('.detail__line').text()).toContain('person@example.com');
    const detailActions = wrapper.findAll('.detail__actions .app-button');
    expect(detailActions.map((button) => button.text())).toEqual([
      'Open as draft',
      'Close warning',
    ]);
    expect(detailActions[1]!.attributes('disabled')).toBeUndefined();
  });

  it('states a failure in the row and keeps queued work quiet', async () => {
    harness.outbox!.items = [
      makeItem({ id: 'failed', status: 'failed', retry_count: 2, last_error: 'Target folder no longer exists' }),
      makeItem({ id: 'queued', status: 'pending', subject: 'Queued mail', recipients: ['a@example.com', 'b@example.com'] }),
    ];
    const wrapper = render();
    const [failedRow, queuedRow] = wrapper.findAll('.row');

    expect(failedRow!.get('.row__reason').text()).toBe('Target folder no longer exists');
    expect(failedRow!.findAll('.row__fix .app-button').map((button) => button.text())).toEqual([
      'Send again',
      'Discard',
    ]);
    expect(failedRow!.get('.row__attempts').text()).toBe('2 attempts');
    expect(failedRow!.get('.row__marker').classes()).toContain('row__marker--danger');

    expect(queuedRow!.get('.row__who').text()).toBe('To a@example.com, b@example.com');
    expect(queuedRow!.find('.row__fix').exists()).toBe(false);
    expect(queuedRow!.find('.row__marker').exists()).toBe(false);
    expect(queuedRow!.find('.row__quick .icon-button[title="Discard"]').exists()).toBe(true);

    expect(failedRow!.attributes('aria-current')).toBe('true');
    await queuedRow!.trigger('click');
    expect(queuedRow!.attributes('aria-current')).toBe('true');
    expect(failedRow!.attributes('aria-current')).toBeUndefined();
  });

  it('retries a failed item once at a time and confirms success', async () => {
    harness.outbox!.items = [makeItem({ id: 'failed', status: 'failed', last_error: 'timeout' })];
    let resolveRetry!: () => void;
    harness.outbox!.retry.mockReturnValueOnce(new Promise<void>((resolve) => {
      resolveRetry = resolve;
    }));
    const wrapper = render();
    const retryButton = wrapper.get('.row .app-button');

    await retryButton.trigger('click');
    await nextTick();
    expect(harness.outbox!.retry).toHaveBeenCalledOnce();
    expect(harness.outbox!.retry).toHaveBeenCalledWith('failed');
    expect(retryButton.attributes('disabled')).toBeDefined();
    expect(wrapper.get('.row').classes()).toContain('row--busy');

    await retryButton.trigger('click');
    expect(harness.outbox!.retry).toHaveBeenCalledOnce();

    resolveRetry();
    await flushPromises();

    expect(harness.toast.success).toHaveBeenCalledWith('Queued again.');
    expect(retryButton.attributes('disabled')).toBeUndefined();
    expect(wrapper.get('.row').classes()).not.toContain('row--busy');
  });

  it('guards cancellation while busy and reports a failed discard', async () => {
    harness.outbox!.items = [makeItem({ id: 'pending', status: 'pending' })];
    let rejectCancel!: (error: Error) => void;
    harness.outbox!.cancel.mockReturnValueOnce(new Promise<void>((_, reject) => {
      rejectCancel = reject;
    }));
    const wrapper = render();
    const discardButton = wrapper.get('.row .icon-button[title="Discard"]');

    await discardButton.trigger('click');
    await nextTick();
    expect(harness.outbox!.cancel).toHaveBeenCalledOnce();
    expect(harness.outbox!.cancel).toHaveBeenCalledWith('pending');
    expect(discardButton.attributes('disabled')).toBeDefined();

    await discardButton.trigger('click');
    expect(harness.outbox!.cancel).toHaveBeenCalledOnce();

    rejectCancel(new Error('offline'));
    await flushPromises();

    expect(harness.toast.error).toHaveBeenCalledWith('Could not discard it: Error: offline');
    expect(discardButton.attributes('disabled')).toBeUndefined();
  });

  it('rescues the text of an ambiguous mail into the composer', async () => {
    harness.outbox!.items = [
      makeItem({
        id: 'unknown',
        status: 'delivery_unknown',
        submission_started_at: '2026-08-30T10:00:01.000Z',
        subject: 'Angebot',
        recipients: ['a@x.de'],
        cc: ['c@x.de'],
        bcc: ['b@x.de'],
        body: 'Guten Tag,',
        body_html: '<p>Guten Tag,</p>',
        attachments: [{ filename: 'offer.pdf', mime_type: 'application/pdf', size: 12 }],
      }),
    ];

    const wrapper = render();
    await nextTick();

    await wrapper.get('[data-test="outbox-open-as-draft"]').trigger('click');

    expect(harness.composerOpen).toHaveBeenCalledWith({
      mode: 'new',
      to: ['a@x.de'],
      cc: ['c@x.de'],
      bcc: ['b@x.de'],
      subject: 'Angebot',
      body: 'Guten Tag,',
      bodyHtml: '<p>Guten Tag,</p>',
    });
    expect(harness.toast.info).toHaveBeenCalledWith(
      'The draft is open. 1 attachment was not kept — add it again.',
    );
  });

  it('offers a retry for a send that crashed before the provider saw it', async () => {
    harness.outbox!.items = [
      makeItem({ id: 'untransmitted', status: 'delivery_unknown', submission_started_at: null }),
    ];

    const wrapper = render();
    await nextTick();

    expect(wrapper.find('.detail__actions .app-button').text()).toBe('Send now');
  });

  it('loads on mount and distinguishes loading from an empty queue', async () => {
    harness.outbox!.loading = true;
    const wrapper = render();

    expect(harness.outbox!.load).toHaveBeenCalledOnce();
    expect(wrapper.get('[data-test="skeleton"]').attributes('data-count')).toBe('3');
    expect(wrapper.find('.outbox__body [data-test="empty-state"]').exists()).toBe(false);

    harness.outbox!.loading = false;
    await nextTick();

    expect(wrapper.get('.outbox__body [data-test="empty-state"] strong').text()).toBe('The outbox is empty');
  });

  it('shows a failed load instead of the empty state and reloads on retry', async () => {
    harness.outbox!.error = 'db is locked';
    const wrapper = render();

    const errorState = wrapper.get('.outbox__body [data-test="error-state"]');
    expect(errorState.attributes('role')).toBe('alert');
    expect(errorState.get('strong').text()).toBe('The outbox could not be loaded.');
    expect(errorState.get('span').text()).toBe('db is locked');
    expect(wrapper.find('.outbox__body [data-test="empty-state"]').exists()).toBe(false);
    expect(wrapper.get('[data-test="pane-header"]').attributes('data-subtitle')).toBe(
      'The outbox could not be loaded.',
    );

    harness.outbox!.load.mockClear();
    await errorState.get('[data-test="error-retry"]').trigger('click');

    expect(harness.outbox!.load).toHaveBeenCalledOnce();
  });
});
