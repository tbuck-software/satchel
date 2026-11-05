<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useInvitesStore } from '@/stores/invites';
import {
  filterInvitesWithoutMirroredEvents,
  formatTime,
  itemFromEvent,
  itemFromInvite,
  sourceColorMap,
  type CalendarItem,
} from '@/utils/calendarGrid';
import { todayAgenda } from '@/utils/todayAgenda';

const invites = useInvitesStore();
const { locale } = useI18n();

/** Timed rows; the all-day line comes below them. */
const MAX_TIMED_ROWS = 3;

const now = ref(new Date());
let clock: ReturnType<typeof setInterval> | null = null;

// Async-mounted with the card, so the calendar tables are not three backend
// calls standing between the app starting and the first thread row.
onMounted(() => {
  void invites.load();
  // What is "now" and what is over moves with the clock.
  clock = setInterval(() => {
    now.value = new Date();
  }, 60_000);
});

onBeforeUnmount(() => {
  if (clock) clearInterval(clock);
});

const items = computed<CalendarItem[]>(() => [
  ...invites.events.map(itemFromEvent),
  ...filterInvitesWithoutMirroredEvents(invites.events, invites.items)
    .map(itemFromInvite)
    .filter((item): item is CalendarItem => item !== null),
]);

const agenda = computed(() => todayAgenda(items.value, now.value, MAX_TIMED_ROWS));

const colors = computed(() =>
  sourceColorMap([...agenda.value.allDay, ...agenda.value.timed.map((row) => row.item)].map((item) => item.sourceKey)),
);

function colorOf(item: CalendarItem): string {
  return item.color ?? colors.value.get(item.sourceKey) ?? 'var(--accent-primary)';
}

/** All-day entries as one line: "Ole in Kur, Kochtag, …". */
const allDayLabel = computed(() =>
  new Intl.ListFormat(locale.value, { style: 'narrow', type: 'conjunction' }).format(
    agenda.value.allDay.map((item) => item.title),
  ),
);

const visible = computed(() => agenda.value.allDay.length > 0 || agenda.value.timed.length > 0);
</script>

<template>
  <section v-if="visible" class="agenda" :aria-label="$t('nav.agenda.title')">
    <span class="agenda__title">{{ $t('nav.agenda.title') }}</span>
    <!--
      One grid for all rows: the time column takes the width of its widest
      entry, so the all-day line says so ("All day" / "Ganztägig") and its
      titles still start on the same edge as the timed rows.
    -->
    <div class="agenda__rows">
      <div
        v-for="row in agenda.timed"
        :key="row.item.key"
        class="agenda__row"
        :class="{ 'agenda__row--current': row.current }"
      >
        <span class="agenda__bar" :style="{ background: colorOf(row.item) }" aria-hidden="true" />
        <span class="agenda__time">
          {{ row.current ? $t('nav.agenda.now') : formatTime(row.item.start) }}
        </span>
        <span class="agenda__label" :title="row.item.title">{{ row.item.title }}</span>
      </div>
      <div v-if="agenda.allDay.length > 0" class="agenda__row agenda__row--all-day">
        <span
          class="agenda__bar agenda__bar--all-day"
          :style="{ background: colorOf(agenda.allDay[0]!) }"
          aria-hidden="true"
        />
        <span class="agenda__time">{{ $t('calendar.event.allDay') }}</span>
        <!-- The count survives a long first title that the ellipsis cuts. -->
        <span class="agenda__label agenda__label--all-day" :title="allDayLabel">
          <span class="agenda__label-text">{{ allDayLabel }}</span>
          <span v-if="agenda.allDay.length > 1" class="agenda__count">
            {{ $t('nav.agenda.allDayCount', { count: agenda.allDay.length }) }}
          </span>
        </span>
      </div>
    </div>
    <span v-if="agenda.moreTimed > 0" class="agenda__more">
      {{ $t('nav.agenda.more', { count: agenda.moreTimed }) }}
    </span>
  </section>
</template>

<style scoped>
.agenda {
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
  min-width: 0;
  overflow: hidden;
  margin: 0 var(--space-sm) var(--space-sm);
  padding: var(--space-2-5) var(--space-md);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  box-shadow: var(--shadow-sm);
}

.agenda__title {
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.agenda__rows {
  display: grid;
  grid-template-columns: auto auto minmax(0, 1fr);
  align-items: center;
  column-gap: var(--space-sm);
  row-gap: var(--space-sm);
  min-width: 0;
  font-size: var(--font-size-xs);
}

.agenda__row {
  display: contents;
}

.agenda__bar {
  width: 3px;
  height: 16px;
  flex-shrink: 0;
  border-radius: var(--radius-xs);
}

/* Covers the row the way the event covers the day. */
.agenda__bar--all-day {
  width: 5px;
}

.agenda__time {
  white-space: nowrap;
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
}

.agenda__label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-primary);
  font-weight: var(--weight-medium);
}

/* The appointment that is running now: its time reads "Now" in the accent. */
.agenda__row--current .agenda__time {
  color: var(--text-accent);
  font-weight: var(--weight-semibold);
}

/* The day's backdrop, quieter than the appointments above it. */
.agenda__label--all-day {
  display: flex;
  align-items: baseline;
  gap: var(--space-1-5);
  color: var(--text-secondary);
  font-weight: var(--weight-regular);
}

.agenda__label-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.agenda__count {
  flex-shrink: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-variant-numeric: tabular-nums;
}

.agenda__more {
  font-size: var(--font-size-2xs);
  color: var(--text-tertiary);
}
</style>
