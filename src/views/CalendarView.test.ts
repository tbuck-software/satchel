// @vitest-environment jsdom

import { defineComponent, nextTick, reactive, type PropType } from 'vue';
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  BackendCalendarEvent,
  BackendCalendarInvite,
  BackendCalendarSource,
} from '@/types/calendar';
import type { CalendarItem } from '@/utils/calendarGrid';
import type { ConfirmRequest } from '@/composables/useConfirm';
import { answerConfirms, type DialogStub } from '@/test/dialogs';

type InvitesHarness = {
  items: BackendCalendarInvite[];
  events: BackendCalendarEvent[];
  sources: BackendCalendarSource[];
  load: ReturnType<typeof vi.fn>;
  syncRemote: ReturnType<typeof vi.fn>;
  createEvent: ReturnType<typeof vi.fn>;
  deleteEvent: ReturnType<typeof vi.fn>;
  respond: ReturnType<typeof vi.fn>;
  importIcs: ReturnType<typeof vi.fn>;
};

type AccountsHarness = {
  emailAccounts: Array<{ id: string; email: string }>;
  activeAccountId: string;
  load: ReturnType<typeof vi.fn>;
};

type CalendarUiHarness = {
  anchor: Date;
  viewMode: 'month' | 'week' | 'day';
  hiddenSources: string[];
  setAnchor: ReturnType<typeof vi.fn>;
  setViewMode: ReturnType<typeof vi.fn>;
  isSourceHidden: ReturnType<typeof vi.fn>;
};

const harness = vi.hoisted(() => ({
  invites: null as InvitesHarness | null,
  accounts: null as AccountsHarness | null,
  calendarUi: null as CalendarUiHarness | null,
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

vi.mock('@/stores/invites', () => ({
  useInvitesStore: () => harness.invites,
}));

vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => harness.accounts,
}));

vi.mock('@/stores/calendarUi', () => ({
  useCalendarUiStore: () => harness.calendarUi,
}));

vi.mock('@/composables/useToast', () => ({
  useToast: () => harness.toast,
}));

import CalendarView from './CalendarView.vue';

enableAutoUnmount(afterEach);

const AppButtonStub = defineComponent({
  name: 'AppButton',
  inheritAttrs: false,
  props: {
    disabled: Boolean,
    title: { type: String, default: undefined },
  },
  emits: ['click'],
  template:
    '<button class="app-button" v-bind="$attrs" :title="title" :disabled="disabled" @click="$emit(\'click\', $event)"><slot /></button>',
});

const IconButtonStub = defineComponent({
  name: 'IconButton',
  inheritAttrs: false,
  props: {
    disabled: Boolean,
    title: { type: String, default: undefined },
  },
  emits: ['click'],
  template:
    '<button class="icon-button" v-bind="$attrs" :title="title" :aria-label="title" :disabled="disabled" @click="$emit(\'click\', $event)"><slot /></button>',
});

const SegmentedControlStub = defineComponent({
  name: 'SegmentedControl',
  props: {
    options: { type: Array as PropType<Array<{ value: string; label: string }>>, required: true },
    modelValue: { type: String, required: true },
  },
  emits: ['update:modelValue'],
  template: `
    <div data-test="view-mode-control">
      <button
        v-for="option in options"
        :key="option.value"
        type="button"
        :data-view-mode="option.value"
        :aria-pressed="option.value === modelValue"
        @click="$emit('update:modelValue', option.value)"
      >{{ option.label }}</button>
    </div>
  `,
});

const CalendarMonthGridStub = defineComponent({
  name: 'CalendarMonthGrid',
  props: {
    anchor: { type: Date, required: true },
    items: { type: Array as PropType<CalendarItem[]>, required: true },
  },
  emits: ['select', 'createAt', 'openDay'],
  template: `
    <div data-test="month-grid" :data-item-keys="items.map((item) => item.key).join(',')">
      <button
        v-if="items.length > 0"
        type="button"
        data-test="select-item"
        @click="$emit('select', items[0], { left: 10, right: 10, top: 10, bottom: 10 })"
      >Select item</button>
      <button
        type="button"
        data-test="open-day"
        @click="$emit('openDay', new Date(2026, 4, 20))"
      >Open day</button>
    </div>
  `,
});

const CalendarWeekGridStub = defineComponent({
  name: 'CalendarWeekGrid',
  props: {
    days: { type: Array as PropType<Date[]>, required: true },
    items: { type: Array as PropType<CalendarItem[]>, required: true },
  },
  emits: ['select', 'createAt', 'openDay'],
  template: `
    <div data-test="week-grid" :data-day-count="String(days.length)" :data-item-keys="items.map((item) => item.key).join(',')" />
  `,
});

const CalendarPopoverStub = defineComponent({
  name: 'CalendarPopover',
  props: {
    anchor: { type: Object as PropType<Record<string, number> | null>, default: null },
  },
  template: '<div v-if="anchor" data-test="calendar-popover"><slot /></div>',
});

const EventDetailsPopoverStub = defineComponent({
  name: 'EventDetailsPopover',
  props: {
    item: { type: Object as PropType<CalendarItem>, required: true },
  },
  emits: ['close', 'respond', 'edit', 'remove'],
  template: `
    <div data-test="event-details">
      <button type="button" data-test="respond" @click="$emit('respond', item, 'accepted')">Respond</button>
      <button type="button" data-test="remove" @click="$emit('remove', item)">Remove</button>
    </div>
  `,
});

const EventEditorDialogStub = defineComponent({
  name: 'EventEditorDialog',
  props: {
    draft: { type: Object as PropType<Record<string, unknown> | null>, default: null },
    saving: Boolean,
  },
  emits: ['close', 'save'],
  setup(props, { emit }) {
    function save() {
      const draft = props.draft;
      if (!draft) throw new Error('Expected an event editor draft');
      emit('save', {
        id: null,
        title: 'Planning session',
        location: null,
        description: null,
        allDay: false,
        startsAt: (draft.start as Date).toISOString(),
        endsAt: (draft.end as Date).toISOString(),
        attendees: [],
        sendInvites: false,
      });
    }

    return { props, save };
  },
  template: `
    <div v-if="props.draft" data-test="event-editor">
      <button type="button" data-test="editor-save" @click="save">Save</button>
    </div>
  `,
});

const DrawerButtonStub = defineComponent({ name: 'DrawerButton', template: '<span />' });

function makeEvent(overrides: Partial<BackendCalendarEvent> = {}): BackendCalendarEvent {
  return {
    id: 'event-1',
    account_id: 'account-1',
    uid: 'uid-1',
    title: 'Planning',
    description: null,
    location: null,
    starts_at: '2026-05-13T10:00:00.000Z',
    ends_at: '2026-05-13T11:00:00.000Z',
    all_day: 0,
    attendees_json: '[]',
    source: 'local',
    collection_href: null,
    created_at: '2026-05-01T00:00:00.000Z',
    updated_at: '2026-05-01T00:00:00.000Z',
    ...overrides,
  };
}

function makeInvite(overrides: Partial<BackendCalendarInvite> = {}): BackendCalendarInvite {
  return {
    id: 'invite-1',
    message_id: 'message-1',
    account_id: 'account-1',
    uid: 'invite-uid-1',
    organizer_email: 'organizer@example.com',
    organizer_name: 'Organizer',
    summary: 'Review meeting',
    description: null,
    location: null,
    starts_at: '2026-05-13T12:00:00.000Z',
    ends_at: '2026-05-13T13:00:00.000Z',
    all_day: 0,
    rsvp_status: 'needs-action',
    method: 'REQUEST',
    cancelled: 0,
    recurrence_id: null,
    recurring: 0,
    recurrence_unbounded: 0,
    raw_ics: 'BEGIN:VCALENDAR\nEND:VCALENDAR',
    sequence: 0,
    created_at: '2026-05-01T00:00:00.000Z',
    updated_at: '2026-05-01T00:00:00.000Z',
    ...overrides,
  };
}

function setupHarness() {
  harness.invites = reactive({
    items: [] as BackendCalendarInvite[],
    events: [] as BackendCalendarEvent[],
    sources: [] as BackendCalendarSource[],
    load: vi.fn().mockResolvedValue(undefined),
    syncRemote: vi.fn().mockResolvedValue([]),
    createEvent: vi.fn().mockResolvedValue(makeEvent()),
    deleteEvent: vi.fn().mockResolvedValue(undefined),
    respond: vi.fn().mockResolvedValue(undefined),
    importIcs: vi.fn().mockResolvedValue(makeInvite({ summary: 'Imported meeting' })),
  }) as InvitesHarness;

  harness.accounts = reactive({
    emailAccounts: [{ id: 'account-1', email: 'one@example.com' }],
    activeAccountId: 'account-1',
    load: vi.fn().mockResolvedValue(undefined),
  }) as AccountsHarness;

  const calendarUi = reactive({
    anchor: new Date(2026, 4, 13),
    viewMode: 'month' as const,
    hiddenSources: [] as string[],
    setAnchor: vi.fn(),
    setViewMode: vi.fn(),
    isSourceHidden: vi.fn(),
  }) as CalendarUiHarness;
  calendarUi.setAnchor.mockImplementation((date: Date) => {
    calendarUi.anchor = date;
  });
  calendarUi.setViewMode.mockImplementation((mode: 'month' | 'week' | 'day') => {
    calendarUi.viewMode = mode;
  });
  calendarUi.isSourceHidden.mockImplementation((key: string) =>
    calendarUi.hiddenSources.includes(key),
  );
  harness.calendarUi = calendarUi;

  harness.toast.success.mockReset();
  harness.toast.error.mockReset();
}

function render() {
  return mount(CalendarView, {
    global: {
      stubs: {
        AppButton: AppButtonStub,
        CalendarMonthGrid: CalendarMonthGridStub,
        CalendarPopover: CalendarPopoverStub,
        CalendarWeekGrid: CalendarWeekGridStub,
        DrawerButton: DrawerButtonStub,
        EventDetailsPopover: EventDetailsPopoverStub,
        EventEditorDialog: EventEditorDialogStub,
        IconButton: IconButtonStub,
        'i18n-t': true,
        QuickCreatePopover: true,
        SegmentedControl: SegmentedControlStub,
      },
    },
  });
}

describe('CalendarView', () => {
  beforeEach(() => {
    setupHarness();
  });

  it('keeps the creation hint on New event instead of below the grid', () => {
    const wrapper = render();
    expect(wrapper.find('.calendar__hint').exists()).toBe(false);
    expect(wrapper.get('button[title="Click an empty day or time slot to create an event."]').text())
      .toBe('New event');
  });

  it('loads local mirrors and starts remote sync, surfacing the first sync error', async () => {
    harness.invites!.syncRemote.mockResolvedValue(['Google calendar unavailable', 'CalDAV unavailable']);

    render();
    await flushPromises();

    expect(harness.invites!.load).toHaveBeenCalledOnce();
    expect(harness.accounts!.load).toHaveBeenCalledOnce();
    expect(harness.invites!.syncRemote).toHaveBeenCalledOnce();
    expect(harness.toast.error).toHaveBeenCalledWith('Google calendar unavailable');
  });

  it('navigates by month, week, and day and switches the selected date to day view', async () => {
    const wrapper = render();

    await wrapper.get('.calendar__nav .icon-button[title="Forward (→)"]').trigger('click');
    expect(harness.calendarUi!.anchor).toEqual(new Date(2026, 5, 1));

    await wrapper.get('[data-view-mode="week"]').trigger('click');
    await nextTick();
    await wrapper.get('.calendar__nav .icon-button[title="Forward (→)"]').trigger('click');
    expect(harness.calendarUi!.anchor).toEqual(new Date(2026, 5, 8));

    await wrapper.get('[data-view-mode="day"]').trigger('click');
    await nextTick();
    await wrapper.get('.calendar__nav .icon-button[title="Forward (→)"]').trigger('click');
    expect(harness.calendarUi!.anchor).toEqual(new Date(2026, 5, 9));

    await wrapper.get('[data-view-mode="month"]').trigger('click');
    await nextTick();
    await wrapper.get('[data-test="open-day"]').trigger('click');
    expect(harness.calendarUi!.setAnchor).toHaveBeenLastCalledWith(new Date(2026, 4, 20));
    expect(harness.calendarUi!.setViewMode).toHaveBeenLastCalledWith('day');
  });

  it('passes only visible source items to the active grid', async () => {
    harness.invites!.events = [
      makeEvent({ id: 'local-event', uid: 'local-uid', source: 'local' }),
      makeEvent({
        id: 'google-event',
        uid: 'google-uid',
        source: 'google',
        collection_href: 'primary',
      }),
    ];
    const wrapper = render();

    expect(wrapper.get('[data-test="month-grid"]').attributes('data-item-keys')).toBe(
      'event:local-event,event:google-event',
    );

    harness.calendarUi!.hiddenSources.push('local');
    await nextTick();

    expect(wrapper.get('[data-test="month-grid"]').attributes('data-item-keys')).toBe(
      'event:google-event',
    );
    expect(harness.calendarUi!.isSourceHidden).toHaveBeenCalledWith('local');
  });

  it('hands an accepted response from event details to the invites store', async () => {
    harness.invites!.items = [makeInvite()];
    const wrapper = render();

    await wrapper.get('[data-test="select-item"]').trigger('click');
    await nextTick();
    await wrapper.get('[data-test="respond"]').trigger('click');
    await flushPromises();

    expect(harness.invites!.respond).toHaveBeenCalledWith('invite-1', 'accepted');
    expect(harness.toast.success).toHaveBeenCalledWith('Acceptance sent.');
  });

  describe('deleting an event', () => {
    let confirms: DialogStub<ConfirmRequest, boolean>;

    beforeEach(() => {
      confirms = answerConfirms(false);
    });

    afterEach(() => confirms.stop());

    async function openEventDetails() {
      harness.invites!.events = [makeEvent()];
      const wrapper = render();
      await wrapper.get('[data-test="select-item"]').trigger('click');
      await nextTick();
      return wrapper;
    }

    it('asks first and keeps the event and its details when the answer is no', async () => {
      const wrapper = await openEventDetails();

      await wrapper.get('[data-test="remove"]').trigger('click');
      await flushPromises();

      expect(confirms.asked).toHaveBeenCalledOnce();
      expect(confirms.asked.mock.calls[0]![0].tone).toBe('danger');
      expect(harness.invites!.deleteEvent).not.toHaveBeenCalled();
      expect(wrapper.find('[data-test="event-details"]').exists()).toBe(true);
    });

    it('deletes the event once it is confirmed', async () => {
      confirms.reply(true);
      const wrapper = await openEventDetails();

      await wrapper.get('[data-test="remove"]').trigger('click');
      await flushPromises();

      expect(harness.invites!.deleteEvent).toHaveBeenCalledOnce();
      expect(wrapper.find('[data-test="event-details"]').exists()).toBe(false);
    });
  });

  it('saves a new event for the active email account', async () => {
    const wrapper = render();

    const newEventButton = wrapper.findAll('.app-button').find((button) => button.text() === 'New event');
    expect(newEventButton).toBeDefined();
    await newEventButton!.trigger('click');
    await nextTick();
    await wrapper.get('[data-test="editor-save"]').trigger('click');
    await flushPromises();

    expect(harness.invites!.createEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        id: null,
        accountId: 'account-1',
        title: 'Planning session',
        allDay: false,
        attendees: [],
        sendInvites: false,
      }),
    );
    expect(harness.toast.success).toHaveBeenCalledWith('Event created.');
    expect(wrapper.find('[data-test="event-editor"]').exists()).toBe(false);
  });

  it('imports an ICS file and reports the imported summary', async () => {
    const wrapper = render();
    const file = new File(['BEGIN:VCALENDAR\nEND:VCALENDAR'], 'meeting.ics', {
      type: 'text/calendar',
    });
    Object.defineProperty(file, 'text', {
      configurable: true,
      value: vi.fn().mockResolvedValue('BEGIN:VCALENDAR\nEND:VCALENDAR'),
    });
    const input = wrapper.get('input[type="file"]').element as HTMLInputElement;
    Object.defineProperty(input, 'files', { configurable: true, value: [file] });

    await wrapper.get('input[type="file"]').trigger('change');
    await flushPromises();

    expect(harness.invites!.importIcs).toHaveBeenCalledWith({
      raw: 'BEGIN:VCALENDAR\nEND:VCALENDAR',
      sourceFilename: 'meeting.ics',
    });
    expect(harness.toast.success).toHaveBeenCalledWith('Imported: Imported meeting');
  });
});
