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

const props = defineProps<{
  anchor: Date;
  items: CalendarItem[];
}>();

const emit = defineEmits<{
  select: [item: CalendarItem, anchor: AnchorBox];
  createAt: [date: Date, anchor: AnchorBox];
  openDay: [date: Date];
}>();

const { locale } = useI18n();

const weekdayHeaders = computed(() => weekdayLabels(locale.value));
const MAX_VISIBLE = 3;

const today = new Date();

const cells = computed(() =>
  monthGridDays(props.anchor).map((day) => {
    const dayItems = itemsForDay(props.items, day);
    return {
      day,
      key: day.toISOString(),
      inMonth: day.getMonth() === props.anchor.getMonth(),
      isToday: isSameDay(day, today),
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
  <div class="month" role="grid" :aria-label="$t('calendar.view.monthGrid')">
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
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  border: var(--border-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  overflow: hidden;
}

.month__weekdays {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  border-bottom: var(--border-muted);
  background: var(--bg-surface);
}

.month__weekdays span {
  padding: 6px 8px;
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
  grid-template-columns: repeat(7, 1fr);
  grid-template-rows: repeat(var(--rows), minmax(0, 1fr));
}

.month__cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-height: 0;
  min-width: 0;
  padding: 4px;
  border-right: var(--border-muted);
  border-bottom: var(--border-muted);
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
  background: color-mix(in srgb, var(--bg-inset) 45%, transparent);
}

.month__cell--muted .month__daynum {
  color: var(--text-tertiary);
}

.month__daynum {
  align-self: flex-end;
  display: grid;
  place-items: center;
  min-width: 24px;
  height: 24px;
  padding: 0 4px;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
  font-variant-numeric: tabular-nums;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.month__daynum:hover {
  background: var(--bg-hover);
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
</style>
