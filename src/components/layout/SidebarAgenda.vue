<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { useInvitesStore } from '@/stores/invites';
import {
  formatTime,
  itemFromEvent,
  itemFromInvite,
  sourceColorMap,
  type CalendarItem,
} from '@/utils/calendarGrid';

const invites = useInvitesStore();

// Async-mounted with the card, so the calendar tables are not three backend
// calls standing between the app starting and the first thread row.
onMounted(() => {
  void invites.load();
});

const MAX_ROWS = 3;

const today = computed(() => {
  const start = new Date();
  start.setHours(0, 0, 0, 0);
  const end = start.getTime() + 24 * 60 * 60 * 1000;
  const items: CalendarItem[] = [
    ...invites.events.map(itemFromEvent),
    ...invites.items.map(itemFromInvite).filter((item): item is CalendarItem => item !== null),
  ];
  return items
    .filter((item) => item.start.getTime() < end && item.end.getTime() > start.getTime())
    .sort((a, b) => a.start.getTime() - b.start.getTime());
});

const colors = computed(() => sourceColorMap(today.value.map((item) => item.sourceKey)));

const rows = computed(() => today.value.slice(0, MAX_ROWS));
const overflow = computed(() => Math.max(0, today.value.length - MAX_ROWS));
</script>

<template>
  <section v-if="today.length > 0" class="agenda" :aria-label="$t('nav.agenda.title')">
    <span class="agenda__title">{{ $t('nav.agenda.title') }}</span>
    <div v-for="item in rows" :key="item.key" class="agenda__row">
      <span
        class="agenda__bar"
        :class="{ 'agenda__bar--all-day': item.allDay }"
        :style="{ background: item.color ?? colors.get(item.sourceKey) ?? 'var(--accent-primary)' }"
        aria-hidden="true"
      />
      <!--
        An all-day row prints no time — the same answer the week and month
        grids give (`show-time="!item.allDay"`). "Ganztägig" is three times
        the width the column has, and it ran straight into the title. The
        bar carries the meaning for the eye, the hidden label for a reader.
      -->
      <span class="agenda__time">
        <span v-if="item.allDay" class="sr-only">{{ $t('calendar.event.allDay') }}</span>
        <template v-else>{{ formatTime(item.start) }}</template>
      </span>
      <span class="agenda__label">{{ item.title }}</span>
    </div>
    <span v-if="overflow > 0" class="agenda__more">{{ $t('nav.agenda.more', { count: overflow }) }}</span>
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

.agenda__row {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  min-width: 0;
  font-size: var(--font-size-xs);
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
  width: 38px;
  flex-shrink: 0;
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

.agenda__more {
  font-size: var(--font-size-2xs);
  color: var(--text-tertiary);
}
</style>
