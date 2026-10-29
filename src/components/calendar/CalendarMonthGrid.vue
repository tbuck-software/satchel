<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { computed } from 'vue';
import {
  isSameDay,
  itemsForDay,
  monthGridDays,
  weekdayLabels,
  type CalendarItem,
} from '@/utils/calendarGrid';
import type { AnchorBox } from '@/utils/popoverPlacement';
import CalendarEventChip from './CalendarEventChip.vue';
import { useI18n } from 'vue-i18n';

const props = withDefaults(defineProps<{
  anchor: Date;
  items: CalendarItem[];
  /**
   * The phone's grid: a cell is a day number and dots, and tapping it picks
   * the day whose events the agenda below lists. Titles do not fit a seventh
   * of a phone.
   */
  compact?: boolean;
}>(), { compact: false });

const emit = defineEmits<{
  select: [item: CalendarItem, anchor: AnchorBox];
  createAt: [date: Date, anchor: AnchorBox];
  openDay: [date: Date];
  pickDay: [date: Date];
}>();

const { locale } = useI18n();

const weekdayHeaders = computed(() => weekdayLabels(locale.value));
const MAX_VISIBLE = 3;
const narrowWeekdayHeaders = computed(() => weekdayLabels(locale.value, 'narrow'));

const today = new Date();

const cells = computed(() =>
  monthGridDays(props.anchor).map((day) => {
    const dayItems = itemsForDay(props.items, day);
    return {
      day,
      key: day.toISOString(),
      inMonth: day.getMonth() === props.anchor.getMonth(),
      isToday: isSameDay(day, today),
      isPicked: isSameDay(day, props.anchor),
      visible: dayItems.slice(0, MAX_VISIBLE),
      overflow: Math.max(0, dayItems.length - MAX_VISIBLE),
    };
  }),
);

const rowCount = computed(() => cells.value.length / 7);


function createOn(day: Date, event: MouseEvent) {
  const at = new Date(day);
  at.setHours(9, 0, 0, 0);
  const { clientX, clientY } = event;
  emit('createAt', at, { left: clientX, right: clientX, top: clientY, bottom: clientY });
}
</script>

<template>
  <div v-if="compact" class="month month--compact" role="grid" :aria-label="$t('calendar.view.monthGrid')">
    <div class="month__weekdays" role="row">
      <span v-for="(label, index) in narrowWeekdayHeaders" :key="index" role="columnheader">{{ label }}</span>
    </div>
    <div class="month__grid">
      <CardButton
        v-for="cell in cells"
        :key="cell.key"
        class="day"
        :class="{ 'day--muted': !cell.inMonth, 'day--today': cell.isToday, 'day--picked': cell.isPicked }"
        role="gridcell"
        :aria-selected="cell.isPicked"
        :aria-label="cell.day.toLocaleDateString(locale, { weekday: 'long', day: 'numeric', month: 'long' })"
        @click="emit('pickDay', cell.day)"
      >
        <span class="day__num">{{ cell.day.getDate() }}</span>
        <span class="day__dots" aria-hidden="true">
          <span
            v-for="item in cell.visible"
            :key="item.key"
            class="day__dot"
            :style="item.color ? { background: item.color } : undefined"
          />
        </span>
      </CardButton>
    </div>
  </div>
  <div v-else class="month" role="grid" :aria-label="$t('calendar.view.monthGrid')">
    <div class="month__weekdays" role="row">
      <span v-for="label in weekdayHeaders" :key="label" role="columnheader">{{ label }}</span>
    </div>
    <div class="month__grid" :style="{ '--rows': rowCount }">
      <div
        v-for="cell in cells"
        :key="cell.key"
        class="month__cell"
        :class="{ 'month__cell--muted': !cell.inMonth, 'month__cell--today': cell.isToday }"
        role="gridcell"
        @click.self="createOn(cell.day, $event)"
      >
        <CardButton
          class="month__daynum"
          :title="$t('calendar.view.openDay')"
          @click="emit('openDay', cell.day)"
        >
          {{ cell.day.getDate() }}
        </CardButton>
        <div class="month__events" @click.self="createOn(cell.day, $event)">
          <CalendarEventChip
            v-for="item in cell.visible"
            :key="item.key"
            :item="item"
            :show-time="!item.allDay && isSameDay(item.start, cell.day)"
            @select="(item, anchor) => emit('select', item, anchor)"
          />
          <AppButton
            v-if="cell.overflow > 0"
            class="month__more"
            @click="emit('openDay', cell.day)"
            variant="ghost"
            size="sm"
          >
            {{ $t('calendar.more', { count: cell.overflow }) }}
          </AppButton>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.month {
  /* Cell padding plus the day number's own side padding. */
  --month-inset: var(--space-2-5);
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  border: 1px solid var(--calendar-grid-line);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  overflow: hidden;
}

.month__weekdays {
  display: grid;
  grid-template-columns: repeat(7, minmax(0, 1fr));
  border-bottom: 1px solid var(--calendar-grid-line);
  background: var(--bg-surface);
}

/* Weekday and day number share one axis: both end at the cell's right
   edge, inset by --month-inset, the way Apple's month view reads. */
.month__weekdays span {
  padding: 6px var(--month-inset);
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  text-align: right;
}

.month__grid {
  display: grid;
  flex: 1;
  min-height: 0;
  grid-template-columns: repeat(7, minmax(0, 1fr));
  grid-template-rows: repeat(var(--rows), minmax(0, 1fr));
}

.month__cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-height: 0;
  min-width: 0;
  padding: 4px;
  border-right: 1px solid var(--calendar-grid-line);
  border-bottom: 1px solid var(--calendar-grid-line);
  overflow: hidden;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.month__cell:hover {
  background: var(--bg-hover);
}

.month__cell:nth-child(7n) {
  border-right: 0;
}

.month__cell--muted {
  background: var(--bg-base);
}

.month__cell--muted .month__daynum {
  color: var(--text-tertiary);
  /* The tertiary ink has to keep 4.5:1, which on dark leaves it bright; the
     lighter weight is what lets the other month's days step back. */
  font-weight: var(--weight-regular);
}

.month__daynum {
  align-self: flex-end;
  display: grid;
  place-items: center;
  min-width: 24px;
  height: 24px;
  padding: 0 var(--space-1-5);
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-primary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
  font-variant-numeric: tabular-nums;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.month__daynum:hover {
  background: var(--bg-hover);
}

/* Today is a place, not an event: the cell itself is lifted with a faint
   tint and an accent edge, so it still reads when its chips share the
   accent. The number badge stays the anchor. */
.month__cell--today {
  background: color-mix(in srgb, var(--accent-primary) 7%, var(--bg-surface));
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-primary) 60%, transparent);
}

.month__cell--today .month__daynum {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.month__events {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.month__more {
  align-self: flex-start;
  border: 0;
  padding: 0 6px;
  background: transparent;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
  cursor: pointer;
}

.month__more:hover {
  color: var(--text-primary);
}

/* Phone: no lines, no chips. A day is a number and up to three dots; the
   picked day wears the filled circle, today the accent ink. */
.month--compact {
  height: auto;
  border: 0;
  border-radius: 0;
  background: transparent;
  overflow: visible;
}

.month--compact .month__weekdays {
  border-bottom: 0;
  background: transparent;
}

.month--compact .month__weekdays span {
  padding: 0 0 var(--space-2xs);
  text-align: center;
  letter-spacing: 0;
}

.month--compact .month__grid {
  grid-template-rows: none;
  grid-auto-rows: 48px;
}

.day {
  display: grid;
  grid-template-rows: 32px 8px;
  place-items: center;
  align-content: center;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-primary);
}

.day__num {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
  font-variant-numeric: tabular-nums;
}

.day--muted .day__num {
  color: var(--text-tertiary);
}

.day--today .day__num {
  color: var(--accent-primary);
  font-weight: var(--weight-bold);
}

.day--picked .day__num {
  background: var(--text-primary);
  color: var(--bg-surface);
  font-weight: var(--weight-bold);
}

.day--picked.day--today .day__num {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.day__dots {
  display: flex;
  gap: 3px;
}

.day__dot {
  width: 5px;
  height: 5px;
  border-radius: var(--radius-full);
  background: var(--accent-primary);
}
</style>
