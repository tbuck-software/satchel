<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue';
import DrawerButton from '@/components/layout/DrawerButton.vue';
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { useInvitesStore } from '@/stores/invites';
import { useAccountsStore } from '@/stores/accounts';
import { useCalendarUiStore } from '@/stores/calendarUi';
import { useUiStore } from '@/stores/ui';
import { useToast } from '@/composables/useToast';
import { useConfirm } from '@/composables/useConfirm';
import {
  addDays,
  addMonths,
  calendarSourceKey,
  draftFromItem,
  emptyDraft,
  filterInvitesWithoutMirroredEvents,
  itemFromEvent,
  itemFromInvite,
  sourceColorMap,
  startOfDay,
  weekDays,
  type CalendarEventDraft,
  type CalendarItem,
} from '@/utils/calendarGrid';
import type { AnchorBox } from '@/utils/popoverPlacement';
import CalendarMonthGrid from '@/components/calendar/CalendarMonthGrid.vue';
import CalendarDayAgenda from '@/components/calendar/CalendarDayAgenda.vue';
import CalendarWeekGrid from '@/components/calendar/CalendarWeekGrid.vue';
import CalendarPopover from '@/components/calendar/CalendarPopover.vue';
import EventDetailsPopover from '@/components/calendar/EventDetailsPopover.vue';
import QuickCreatePopover from '@/components/calendar/QuickCreatePopover.vue';
import EventEditorDialog, {
  type EventEditorPayload,
} from '@/components/calendar/EventEditorDialog.vue';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import { useI18n } from 'vue-i18n';

const { d, locale, t } = useI18n();

const viewOptions = computed(() => [
  { value: 'month', label: t('calendar.view.month'), title: t('calendar.view.monthTitle') },
  { value: 'week', label: t('calendar.view.week'), title: t('calendar.view.weekTitle') },
  { value: 'day', label: t('calendar.view.day'), title: t('calendar.view.dayTitle') },
]);

const invites = useInvitesStore();
const accounts = useAccountsStore();
const calendarUi = useCalendarUiStore();
const toast = useToast();
const { ask } = useConfirm();

const viewMode = computed(() => calendarUi.viewMode);
const ui = useUiStore();
/** The phone's month: a compact grid plus the picked day's agenda below it. */
const phoneMonth = computed(() => ui.isMobile && viewMode.value === 'month');
const agendaDays = computed(() => [anchor.value]);

function pickDay(day: Date) {
  calendarUi.setAnchor(day);
}
const anchor = computed(() => calendarUi.anchor);
const selected = ref<{ item: CalendarItem; anchor: AnchorBox } | null>(null);
const quick = ref<{ draft: CalendarEventDraft; anchor: AnchorBox } | null>(null);
const editorDraft = ref<CalendarEventDraft | null>(null);
const saving = ref(false);
const filePicker = ref<HTMLInputElement | null>(null);

onMounted(() => {
  invites.load();
  accounts.load();
  window.addEventListener('keydown', onKeydown);
  // Refresh the remote mirrors in the background; surface only real errors.
  void invites
    .syncRemote()
    .then((errors) => {
      const firstError = errors[0];
      if (firstError) toast.error(firstError);
    })
    .catch(() => {
      /* offline or backend busy — the cached mirror stays visible */
    });
});

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown);
});

function setViewMode(value: string) {
  calendarUi.setViewMode(value as 'month' | 'week' | 'day');
}

// Responding to an invite can mirror it into the local events table under the
// same UID — the event wins, the invite chip disappears.
const allItems = computed<CalendarItem[]>(() => {
  const events = invites.events.map(itemFromEvent);
  const inviteItems = filterInvitesWithoutMirroredEvents(invites.events, invites.items)
    .map(itemFromInvite)
    .filter((item): item is CalendarItem => item !== null);
  const merged = [...events, ...inviteItems];
  const colors = sourceColorMap([
    ...merged.map((item) => item.sourceKey),
    ...invites.sources.map((source) => calendarSourceKey(source.source, source.remote_id)),
  ]);
  return merged.map((item) => ({ ...item, color: colors.get(item.sourceKey) ?? null }));
});

/** What the grids render: everything except sidebar-hidden sources. */
const items = computed(() =>
  allItems.value.filter((item) => !calendarUi.isSourceHidden(item.sourceKey)),
);

const undatedInvites = computed(() => invites.items.filter((invite) => !invite.starts_at));

const gridDays = computed(() =>
  viewMode.value === 'day' ? [startOfDay(anchor.value)] : weekDays(anchor.value),
);

const rangeLabel = computed(() => {
  if (viewMode.value === 'month') {
    return new Intl.DateTimeFormat(locale.value, { month: 'long', year: 'numeric' }).format(anchor.value);
  }
  if (viewMode.value === 'day') return d(anchor.value, 'long');
  const [first, , , , , , last] = weekDays(anchor.value);
  if (!first || !last) return '';
  if (first.getMonth() === last.getMonth()) {
    const monthYear = new Intl.DateTimeFormat(locale.value, {
      month: 'long',
      year: 'numeric',
    }).format(first);
    return t('calendar.range.weekSameMonth', {
      first: first.getDate(),
      last: last.getDate(),
      monthYear,
    });
  }
  const firstLabel = first.toLocaleDateString(locale.value, {
    day: 'numeric',
    month: 'short',
    year: first.getFullYear() === last.getFullYear() ? undefined : 'numeric',
  });
  const lastLabel = last.toLocaleDateString(locale.value, {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
  });
  return t('calendar.range.span', { start: firstLabel, end: lastLabel });
});

function navigate(direction: 1 | -1) {
  if (viewMode.value === 'month') calendarUi.setAnchor(addMonths(anchor.value, direction));
  else if (viewMode.value === 'week') calendarUi.setAnchor(addDays(anchor.value, direction * 7));
  else calendarUi.setAnchor(addDays(anchor.value, direction));
}

function goToday() {
  calendarUi.setAnchor(new Date());
}

function openDay(day: Date) {
  calendarUi.setAnchor(day);
  setViewMode('day');
}

function newEvent() {
  const at = new Date();
  at.setMinutes(0, 0, 0);
  at.setHours(at.getHours() + 1);
  editorDraft.value = emptyDraft(at);
}

function selectItem(item: CalendarItem, anchorBox: AnchorBox) {
  quick.value = null;
  selected.value = { item, anchor: anchorBox };
}

function createAt(date: Date, anchorBox: AnchorBox) {
  selected.value = null;
  quick.value = { draft: emptyDraft(date), anchor: anchorBox };
}

function editItem(item: CalendarItem) {
  selected.value = null;
  editorDraft.value = draftFromItem(item);
}

async function quickSave(title: string) {
  if (!quick.value) return;
  const { draft } = quick.value;
  const saved = await saveEvent({
    id: null,
    title,
    location: null,
    description: null,
    allDay: draft.allDay,
    startsAt: draft.start.toISOString(),
    endsAt: draft.end.toISOString(),
    attendees: [],
    sendInvites: false,
  });
  if (saved) quick.value = null;
}

function quickMore(title: string) {
  if (!quick.value) return;
  editorDraft.value = { ...quick.value.draft, title };
  quick.value = null;
}

async function respond(item: CalendarItem, response: 'accepted' | 'declined' | 'tentative') {
  const labels = {
    accepted: t('calendar.toast.accepted'),
    declined: t('calendar.toast.declined'),
    tentative: t('calendar.toast.tentative'),
  } as const;
  try {
    await invites.respond(item.id, response);
    selected.value = null;
    toast.success(labels[response]);
  } catch (err) {
    toast.error(t('calendar.toast.rsvpFailed', { error: err instanceof Error ? err.message : err }));
  }
}

async function respondUndated(inviteId: string, response: 'accepted' | 'declined' | 'tentative') {
  try {
    await invites.respond(inviteId, response);
    toast.success(t('calendar.toast.rsvpSent'));
  } catch (err) {
    toast.error(t('calendar.toast.rsvpFailed', { error: err instanceof Error ? err.message : err }));
  }
}

async function removeItem(item: CalendarItem) {
  // The details card floats above every modal, so it steps aside for the
  // question and comes back if the answer is no.
  const shown = selected.value;
  selected.value = null;
  const confirmed = await ask({
    title: t('calendar.event.deleteConfirmTitle'),
    body: item.source === 'caldav'
      ? t('calendar.event.deleteConfirmRemote')
      : t('calendar.event.deleteConfirmBody'),
    detail: item.title,
    confirmLabel: t('calendar.event.delete'),
    tone: 'danger',
  });
  if (!confirmed) {
    selected.value = shown;
    return;
  }
  try {
    await invites.deleteEvent(item.id);
    selected.value = null;
    toast.success(t('calendar.toast.eventDeleted'));
  } catch (err) {
    toast.error(t('calendar.toast.deleteFailed', { error: err instanceof Error ? err.message : err }));
  }
}

async function saveEvent(payload: EventEditorPayload): Promise<boolean> {
  const accountId =
    accounts.emailAccounts.find((account) => account.id === accounts.activeAccountId)?.id ||
    accounts.emailAccounts[0]?.id;
  if (!accountId) {
    toast.error(t('calendar.toast.noAccount'));
    return false;
  }
  saving.value = true;
  try {
    await invites.createEvent({
      id: payload.id,
      accountId,
      title: payload.title,
      description: payload.description,
      location: payload.location,
      startsAt: payload.startsAt,
      endsAt: payload.endsAt,
      allDay: payload.allDay,
      attendees: payload.attendees,
      sendInvites: payload.sendInvites,
    });
    editorDraft.value = null;
    toast.success(t(payload.id ? 'calendar.toast.eventUpdated' : 'calendar.toast.eventCreated'));
    return true;
  } catch (err) {
    toast.error(t('calendar.toast.saveFailed', { error: err instanceof Error ? err.message : err }));
    return false;
  } finally {
    saving.value = false;
  }
}

async function importIcsFile(file: File) {
  try {
    const raw = await file.text();
    const invite = await invites.importIcs({ raw, sourceFilename: file.name });
    toast.success(t('calendar.toast.imported', { name: invite.summary || invite.uid }));
  } catch (err) {
    toast.error(t('calendar.toast.importFailed', { error: err instanceof Error ? err.message : err }));
  }
}

function onFiles(event: Event) {
  const target = event.target as HTMLInputElement;
  for (const file of Array.from(target.files ?? [])) {
    void importIcsFile(file);
  }
  target.value = '';
}

function isEditableTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName?.toLowerCase();
  return tag === 'input' || tag === 'textarea' || tag === 'select' || el.isContentEditable;
}

function onKeydown(event: KeyboardEvent) {
  if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;
  if (selected.value || quick.value || editorDraft.value) return;
  if (isEditableTarget(event.target)) return;
  if (event.key === 't') goToday();
  else if (event.key === 'ArrowLeft') navigate(-1);
  else if (event.key === 'ArrowRight') navigate(1);
  else if (event.key === '1') setViewMode('month');
  else if (event.key === '2') setViewMode('week');
  else if (event.key === '3') setViewMode('day');
  else return;
  event.preventDefault();
}
</script>

<template>
  <section class="calendar" :class="{ 'calendar--phone': ui.isMobile }">
    <header class="calendar__toolbar">
      <div class="calendar__heading">
        <DrawerButton /><h2 v-if="!ui.isMobile">{{ $t('calendar.title') }}</h2>
        <span class="calendar__range">{{ rangeLabel }}</span>
      </div>
      <!-- A phone has no room for a text toolbar: Import and New event become
           the two icons at the end of the title row, like a native app bar. -->
      <div v-if="ui.isMobile" class="calendar__phone-actions">
        <IconButton :title="$t('calendar.importIcs')" :aria-label="$t('calendar.importIcs')" @click="filePicker?.click()">
          <AppIcon><path d="M8 2v8M4.5 6.5 8 10l3.5-3.5M2.5 11v2.5h11V11" /></AppIcon>
        </IconButton>
        <IconButton tone="accent" :title="$t('calendar.newEvent')" :aria-label="$t('calendar.newEvent')" @click="newEvent">
          <AppIcon><path d="M8 3v10M3 8h10" /></AppIcon>
        </IconButton>
      </div>
      <div class="calendar__nav">
        <IconButton size="sm" :title="$t('calendar.nav.previous')" @click="navigate(-1)"><AppIcon><path d="m10 3-5 5 5 5" /></AppIcon></IconButton>
        <AppButton variant="soft" size="sm" :title="$t('calendar.nav.today')" @click="goToday">{{ $t('common.today') }}</AppButton>
        <IconButton size="sm" :title="$t('calendar.nav.next')" @click="navigate(1)"><AppIcon><path d="m6 3 5 5-5 5" /></AppIcon></IconButton>
      </div>
      <div class="calendar__actions">
        <SegmentedControl
          :model-value="viewMode"
          :options="viewOptions"
          :aria-label="$t('calendar.view.label')"
          @update:model-value="setViewMode"
        />
        <AppButton v-if="!ui.isMobile" variant="ghost" size="sm" :title="$t('calendar.importIcsHint')" @click="filePicker?.click()">
          {{ $t('calendar.importIcs') }}
        </AppButton>
        <AppButton v-if="!ui.isMobile" variant="primary" size="sm" :title="$t('calendar.createHint')" @click="newEvent">{{ $t('calendar.newEvent') }}</AppButton>
        <input
          ref="filePicker"
          type="file"
          accept=".ics,text/calendar"
          multiple
          style="display: none"
          @change="onFiles"
        />
      </div>
    </header>

    <div v-if="undatedInvites.length > 0" class="calendar__undated">
      <p>{{ $t('calendar.undated.heading') }}</p>
      <div v-for="invite in undatedInvites" :key="invite.id" class="calendar__undated-row">
        <span class="calendar__undated-title">{{ invite.summary || $t('calendar.event.untitled') }}</span>
        <span class="calendar__undated-actions">
          <AppButton variant="primary" size="sm" @click="respondUndated(invite.id, 'accepted')">
            {{ $t('calendar.rsvp.accept') }}
          </AppButton>
          <AppButton variant="soft" size="sm" @click="respondUndated(invite.id, 'tentative')">
            {{ $t('calendar.rsvp.maybe') }}
          </AppButton>
          <AppButton variant="danger" size="sm" @click="respondUndated(invite.id, 'declined')">
            {{ $t('calendar.rsvp.decline') }}
          </AppButton>
        </span>
      </div>
    </div>

    <div class="calendar__grid">
      <CalendarMonthGrid
        v-if="viewMode === 'month'"
        :anchor="anchor"
        :items="items"
        :compact="phoneMonth"
        @select="selectItem"
        @create-at="createAt"
        @open-day="openDay"
        @pick-day="pickDay"
      />
      <CalendarDayAgenda
        v-if="phoneMonth"
        class="calendar__agenda"
        :days="agendaDays"
        :items="items"
        @select="selectItem"
      />
      <CalendarWeekGrid
        v-else
        :days="gridDays"
        :items="items"
        @select="selectItem"
        @create-at="createAt"
        @open-day="openDay"
      />
    </div>

    <CalendarPopover
      :anchor="selected?.anchor ?? null"
      :aria-label="$t('calendar.event.details')"
      @close="selected = null"
    >
      <EventDetailsPopover
        v-if="selected"
        :item="selected.item"
        @close="selected = null"
        @respond="respond"
        @edit="editItem"
        @remove="removeItem"
      />
    </CalendarPopover>
    <CalendarPopover
      :anchor="quick?.anchor ?? null"
      :width="300"
      :aria-label="$t('calendar.quick.create')"
      @close="quick = null"
    >
      <QuickCreatePopover
        v-if="quick"
        :key="quick.draft.start.getTime()"
        :draft="quick.draft"
        :saving="saving"
        @save="quickSave"
        @more="quickMore"
      />
    </CalendarPopover>
    <EventEditorDialog
      :draft="editorDraft"
      :saving="saving"
      @close="editorDraft = null"
      @save="saveEvent"
    />
  </section>
</template>

<style scoped>
.calendar {
  display: flex;
  flex-direction: column;
  gap: var(--space-md);
  height: 100%;
  min-height: 0;
  padding: var(--space-lg) var(--space-xl);
}

.calendar__toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-lg);
  flex-wrap: wrap;
}

.calendar__heading {
  display: flex;
  align-items: baseline;
  gap: var(--space-md);
  min-width: 0;
}

.calendar__heading h2 {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  letter-spacing: -0.01em;
}

.calendar__range {
  color: var(--text-secondary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
  white-space: nowrap;
}

.calendar__nav {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
}

.calendar__actions {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  margin-left: auto;
}

.calendar__undated {
  display: grid;
  gap: var(--space-sm);
  border: var(--border-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  padding: var(--space-md) var(--space-lg);
}

.calendar__undated p {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}

.calendar__undated-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-md);
}

.calendar__undated-title {
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}

.calendar__undated-actions {
  display: flex;
  gap: var(--space-xs);
}

.calendar__grid {
  flex: 1;
  min-height: 0;
}

/* Phone: the title row carries the month and two icons; view switch and
   arrows share the row under it. Grid and agenda scroll together. */
.calendar--phone {
  gap: var(--space-sm);
  padding: var(--space-xs) var(--space-md) 0;
}

.calendar--phone .calendar__toolbar {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--space-xs) var(--space-sm);
}

.calendar--phone .calendar__heading {
  align-items: center;
  gap: var(--space-xs);
}

.calendar--phone .calendar__range {
  overflow: hidden;
  color: var(--text-primary);
  font-size: var(--font-size-xl);
  font-weight: var(--weight-bold);
  text-overflow: ellipsis;
}

.calendar__phone-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2xs);
}

.calendar--phone .calendar__actions {
  grid-row: 2;
  grid-column: 1;
  margin-left: 0;
}

.calendar--phone .calendar__nav {
  grid-row: 2;
  grid-column: 2;
}

.calendar--phone .calendar__grid {
  overflow-y: auto;
  padding-bottom: var(--space-lg);
}

.calendar__agenda {
  margin-top: var(--space-md);
}
</style>
