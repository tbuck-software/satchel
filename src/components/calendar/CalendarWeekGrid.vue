<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import {
  allDayItemsForDay,
  formatTime,
  formatWeekday,
  isSameDay,
  layoutDay,
  type CalendarItem,
} from '@/utils/calendarGrid';
import type { AnchorBox } from '@/utils/popoverPlacement';
import CalendarEventChip from './CalendarEventChip.vue';
import { useI18n } from 'vue-i18n';

const props = defineProps<{
  days: Date[];
  items: CalendarItem[];
}>();

const emit = defineEmits<{
  select: [item: CalendarItem, anchor: AnchorBox];
  createAt: [date: Date, anchor: AnchorBox];
  openDay: [date: Date];
}>();

const HOUR_HEIGHT = 48;
const OVERLAP_INSET = 10;
const MAX_OVERLAP_INSET_LEVELS = 3;
/**
 * Below this a block cannot stack title over time (24px for half an hour), so
 * both go on one line. Stacked, the title shrank to nothing and a short event
 * read as a bare time range.
 */
const COMPACT_EVENT_MINUTES = 45;
const SNAP_MINUTES = 30;

const { locale } = useI18n();

const scroller = ref<HTMLElement | null>(null);
const now = ref(new Date());
let nowTimer: ReturnType<typeof setInterval> | null = null;

const hours = Array.from({ length: 24 }, (_, h) => `${String(h).padStart(2, '0')}:00`);

const columns = computed(() =>
  props.days.map((day) => ({
    day,
    key: day.toISOString(),
    label: formatWeekday(day, locale.value),
    isToday: isSameDay(day, now.value),
    allDay: allDayItemsForDay(props.items, day),
    positioned: layoutDay(props.items, day),
  })),
);

const hasAllDayLane = computed(() => columns.value.some((col) => col.allDay.length > 0));

const nowOffset = computed(() => {
  const minutes = now.value.getHours() * 60 + now.value.getMinutes();
  return (minutes / 60) * HOUR_HEIGHT;
});

function blockStyle(entry: { startMin: number; endMin: number; overlapLevel: number }) {
  const inset = Math.min(entry.overlapLevel, MAX_OVERLAP_INSET_LEVELS) * OVERLAP_INSET;
  return {
    top: `${(entry.startMin / 60) * HOUR_HEIGHT}px`,
    height: `${((entry.endMin - entry.startMin) / 60) * HOUR_HEIGHT}px`,
    left: `${inset}px`,
    right: '1px',
  };
}

function onColumnClick(event: MouseEvent, day: Date) {
  const target = event.currentTarget as HTMLElement;
  const offsetY = event.clientY - target.getBoundingClientRect().top;
  const minutes = Math.max(
    0,
    Math.min(24 * 60 - 60, Math.round(offsetY / HOUR_HEIGHT / (SNAP_MINUTES / 60)) * SNAP_MINUTES),
  );
  const at = new Date(day);
  at.setHours(0, minutes, 0, 0);
  const { clientX, clientY } = event;
  emit('createAt', at, { left: clientX, right: clientX, top: clientY, bottom: clientY });
}

function onEventClick(event: MouseEvent, item: CalendarItem) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  emit('select', item, rect);
}

onMounted(() => {
  nowTimer = setInterval(() => {
    now.value = new Date();
  }, 60_000);
  // Land the viewport on the working morning instead of midnight.
  scrollToWorkday();
});

watch(
  () => props.days[0]?.getTime(),
  () => scrollToWorkday(),
);

function scrollToWorkday() {
  if (scroller.value) scroller.value.scrollTop = 7.5 * HOUR_HEIGHT;
}

onBeforeUnmount(() => {
  if (nowTimer) clearInterval(nowTimer);
});
</script>

<template>
  <div class="week" :class="{ 'week--single': days.length === 1 }">
    <div class="week__header">
      <div class="week__corner" aria-hidden="true"></div>
      <div v-for="col in columns" :key="col.key" class="week__dayhead">
        <CardButton
          class="week__daylabel"
          :class="{ 'week__daylabel--today': col.isToday }"
          :disabled="days.length === 1"
          :title="days.length === 1 ? undefined : $t('calendar.view.openDay')"
          @click="emit('openDay', col.day)"
        >
          <span class="week__dayname">{{ col.label }}</span>
          <span class="week__daynum">{{ col.day.getDate() }}</span>
        </CardButton>
        <div v-if="hasAllDayLane" class="week__allday">
          <CalendarEventChip
            v-for="item in col.allDay"
            :key="item.key"
            :item="item"
            :show-time="false"
            @select="(item, anchor) => emit('select', item, anchor)"
          />
        </div>
      </div>
    </div>

    <div ref="scroller" class="week__body">
      <div class="week__times" aria-hidden="true">
        <span
          v-for="hour in hours"
          :key="hour"
          class="week__time"
          :style="{ height: `${HOUR_HEIGHT}px` }"
        >
          {{ hour }}
        </span>
      </div>
      <div class="week__cols">
        <div
          v-for="col in columns"
          :key="col.key"
          class="week__col"
          :style="{ height: `${24 * HOUR_HEIGHT}px` }"
          @click.self="onColumnClick($event, col.day)"
        >
          <CardButton
            v-for="entry in col.positioned"
            :key="entry.item.key"
            class="week__event"
            :class="[
              `week__event--${entry.item.status}`,
              {
                'week__event--overlapping': entry.overlapLevel > 0,
                'week__event--compact': entry.endMin - entry.startMin < COMPACT_EVENT_MINUTES,
              },
            ]"
            :style="{
              ...blockStyle(entry),
              ...(entry.item.color ? { '--chip-accent': entry.item.color } : {}),
            }"
            :title="entry.item.title"
            @click.stop="onEventClick($event, entry.item)"
          >
            <span class="week__event-title">{{ entry.item.title }}</span>
            <span class="week__event-time">
              {{ formatTime(entry.item.start) }}–{{ formatTime(entry.item.end) }}
            </span>
          </CardButton>
          <div
            v-if="col.isToday"
            class="week__now"
            :style="{ top: `${nowOffset}px` }"
            aria-hidden="true"
          ></div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.week {
  --times-width: 56px;
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  border: var(--border-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  overflow: hidden;
}

.week__header {
  display: grid;
  grid-template-columns: var(--times-width) repeat(7, 1fr);
  border-bottom: var(--border-muted);
}

.week--single .week__header {
  grid-template-columns: var(--times-width) 1fr;
}

.week__corner {
  border-right: var(--border-muted);
}

.week__dayhead {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  padding: 6px 6px 4px;
  border-right: var(--border-muted);
}

.week__dayhead:last-child {
  border-right: 0;
}

.week__daylabel {
  display: inline-flex;
  align-items: center;
  align-self: flex-start;
  gap: 6px;
  border: 0;
  border-radius: var(--radius-full);
  padding: 2px 8px;
  background: transparent;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.week__daylabel:hover:not(:disabled) {
  background: var(--bg-hover);
}

.week__daylabel:disabled {
  cursor: default;
}

.week__dayname {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
  text-transform: uppercase;
  letter-spacing: 0.06em;
}

.week__daynum {
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-bold);
  font-variant-numeric: tabular-nums;
}

.week__daylabel--today .week__daynum {
  display: grid;
  place-items: center;
  min-width: 24px;
  height: 24px;
  border-radius: var(--radius-full);
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.week__allday {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-height: 20px;
  padding-bottom: 2px;
}

.week__body {
  display: grid;
  flex: 1;
  min-height: 0;
  grid-template-columns: var(--times-width) 1fr;
  overflow-y: auto;
}

.week__times {
  display: flex;
  flex-direction: column;
  border-right: var(--border-muted);
}

.week__time {
  flex-shrink: 0;
  padding: 2px 8px 0 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-variant-numeric: tabular-nums;
  text-align: right;
  transform: translateY(-0.6em);
}

.week__time:first-child {
  visibility: hidden;
}

.week__cols {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
}

.week--single .week__cols {
  grid-template-columns: 1fr;
}

.week__col {
  position: relative;
  min-width: 0;
  border-right: var(--border-muted);
  cursor: pointer;
  /* Hour ruling without 24 extra nodes per column. */
  background-image: repeating-linear-gradient(
    to bottom,
    var(--border-color-muted) 0 1px,
    transparent 1px 48px
  );
  transition: background-color var(--duration-fast) var(--ease-out);
}

.week__col:hover {
  background-color: color-mix(in srgb, var(--bg-hover) 55%, transparent);
}

.week__col:last-child {
  border-right: 0;
}

.week__event {
  position: absolute;
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  margin: 0 1px;
  padding: 3px 6px;
  border: 0;
  /* Own events are tinted by their source calendar; invites override below. */
  border-left: 2px solid
    color-mix(in srgb, var(--chip-accent, var(--accent-primary)) 72%, var(--text-primary));
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--chip-accent, var(--accent-primary)) 22%, var(--bg-surface));
  color: var(--text-primary);
  font-size: var(--font-size-2xs);
  text-align: left;
  overflow: hidden;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.week__event--overlapping {
  box-shadow: -1px 1px 3px rgba(0, 0, 0, 0.22);
}

.week__event:hover {
  background: color-mix(in srgb, var(--chip-accent, var(--accent-primary)) 34%, var(--bg-surface));
}

.week__event--compact {
  flex-direction: row;
  align-items: center;
  gap: var(--space-1-5);
  padding-block: 0;
  line-height: 1.6;
}

.week__event-title {
  flex-shrink: 0;
  max-width: 100%;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-weight: var(--weight-semibold);
}

.week__event-time {
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.week__event--accepted {
  border-left-color: color-mix(in srgb, var(--accent-green) 72%, var(--text-primary));
  background: color-mix(in srgb, var(--accent-green) 20%, var(--bg-surface));
}
.week__event--accepted:hover {
  background: color-mix(in srgb, var(--accent-green) 32%, var(--bg-surface));
}

.week__event--tentative {
  border-left-color: color-mix(in srgb, var(--accent-amber) 72%, var(--text-primary));
  background: color-mix(in srgb, var(--accent-amber) 20%, var(--bg-surface));
}
.week__event--tentative:hover {
  background: color-mix(in srgb, var(--accent-amber) 32%, var(--bg-surface));
}

.week__event--needs-action {
  border-left-color: color-mix(in srgb, var(--accent-purple) 72%, var(--text-primary));
  background: color-mix(in srgb, var(--accent-purple) 20%, var(--bg-surface));
}
.week__event--needs-action:hover {
  background: color-mix(in srgb, var(--accent-purple) 32%, var(--bg-surface));
}

.week__event--declined,
.week__event--cancelled {
  border-left-color: var(--border-color);
  background: var(--bg-raised);
  color: var(--text-tertiary);
}

.week__event--declined .week__event-title,
.week__event--cancelled .week__event-title {
  text-decoration: line-through;
}

.week__now {
  position: absolute;
  left: 0;
  right: 0;
  z-index: 2;
  height: 2px;
  background: var(--accent-red);
  pointer-events: none;
}

.week__now::before {
  content: '';
  position: absolute;
  left: -3px;
  top: -3px;
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--accent-red);
}
</style>
