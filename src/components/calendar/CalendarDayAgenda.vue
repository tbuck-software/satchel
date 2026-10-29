<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed } from 'vue';
import { formatDayLong, formatTime, isSameDay, itemsForDay, type CalendarItem } from '@/utils/calendarGrid';
import type { AnchorBox } from '@/utils/popoverPlacement';

/**
 * The phone's answer to a month cell: what a day holds, as readable rows.
 *
 * A seventh of a phone is about 50pt, enough for a day number and a few dots
 * but not for a title. Apple Calendar and Google Calendar both put the
 * selected days' events in a list below the grid; this is that list.
 */
const props = defineProps<{
  days: Date[];
  items: CalendarItem[];
}>();

const emit = defineEmits<{
  select: [item: CalendarItem, anchor: AnchorBox];
}>();

const today = new Date();

const sections = computed(() => props.days.map((day) => ({
  key: day.toISOString(),
  day,
  isToday: isSameDay(day, today),
  items: itemsForDay(props.items, day),
})));

function timeLabel(item: CalendarItem, day: Date): string {
  if (item.allDay || !isSameDay(item.start, day)) return '';
  return formatTime(item.start);
}

function onSelect(item: CalendarItem, event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  emit('select', item, rect);
}
</script>

<template>
  <div class="agenda">
    <section v-for="section in sections" :key="section.key" class="agenda__day">
      <h3 class="agenda__heading" :class="{ 'agenda__heading--today': section.isToday }">
        {{ formatDayLong(section.day) }}
      </h3>
      <p v-if="section.items.length === 0" class="agenda__empty">{{ $t('calendar.agenda.empty') }}</p>
      <CardButton
        v-for="item in section.items"
        :key="item.key"
        class="agenda__row"
        :style="item.color ? { '--agenda-accent': item.color } : undefined"
        @click="onSelect(item, $event)"
      >
        <span class="agenda__time">
          {{ timeLabel(item, section.day) || $t('calendar.event.allDay') }}
        </span>
        <span class="agenda__bar" aria-hidden="true" />
        <span class="agenda__text">
          <span class="agenda__title">{{ item.title || $t('calendar.event.untitled') }}</span>
          <span v-if="item.location" class="agenda__location">{{ item.location }}</span>
        </span>
      </CardButton>
    </section>
  </div>
</template>

<style scoped>
.agenda {
  display: grid;
  gap: var(--space-md);
}

.agenda__day {
  display: grid;
  gap: var(--space-2xs);
}

.agenda__heading {
  margin: 0 0 var(--space-2xs);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}

.agenda__heading--today {
  color: var(--accent-primary);
}

.agenda__empty {
  margin: 0;
  padding: var(--space-sm) 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.agenda__row {
  display: grid;
  grid-template-columns: 64px 3px minmax(0, 1fr);
  align-items: center;
  gap: var(--space-sm);
  min-height: 52px;
  padding: var(--space-xs) var(--space-sm);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-primary);
  text-align: left;
}

.agenda__time {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.agenda__bar {
  align-self: stretch;
  border-radius: var(--radius-full);
  background: var(--agenda-accent, var(--accent-primary));
}

.agenda__text {
  display: grid;
  min-width: 0;
}

.agenda__title,
.agenda__location {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.agenda__title {
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
}

.agenda__location {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}
</style>
