<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed, ref, watch } from 'vue';
import {
  addMonths,
  isSameDay,
  monthGridDays,
} from '@/utils/calendarGrid';
import IconButton from '@/components/ui/IconButton.vue';
import { useI18n } from 'vue-i18n';

const props = defineProps<{
  /** The day the main view is anchored on. */
  selected: Date;
}>();

const emit = defineEmits<{
  select: [date: Date];
}>();

const { locale } = useI18n();

// Own month cursor so browsing months does not move the main view; it snaps
// back to the selected day whenever the main view navigates.
const cursor = ref(new Date(props.selected));

const weekdayLabels = computed(() => {
  const monday = new Date(2024, 0, 1);
  return Array.from({ length: 7 }, (_, index) =>
    new Intl.DateTimeFormat(locale.value, { weekday: 'narrow' }).format(
      new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() + index),
    ),
  );
});

const monthTitle = computed(() =>
  new Intl.DateTimeFormat(locale.value, { month: 'long', year: 'numeric' }).format(cursor.value),
);

watch(
  () => props.selected.getTime(),
  () => {
    cursor.value = new Date(props.selected);
  },
);

const today = new Date();

const cells = computed(() =>
  monthGridDays(cursor.value).map((day) => ({
    day,
    key: day.getTime(),
    inMonth: day.getMonth() === cursor.value.getMonth(),
    isToday: isSameDay(day, today),
    isSelected: isSameDay(day, props.selected),
  })),
);
</script>

<template>
  <div class="mini-month">
    <header class="mini-month__header">
      <span class="mini-month__title">{{ monthTitle }}</span>
      <span class="mini-month__nav">
        <IconButton size="sm" :title="$t('calendar.nav.previousMonth')" @click="cursor = addMonths(cursor, -1)">
          ‹
        </IconButton>
        <IconButton size="sm" :title="$t('calendar.nav.nextMonth')" @click="cursor = addMonths(cursor, 1)">
          ›
        </IconButton>
      </span>
    </header>
    <div class="mini-month__weekdays" aria-hidden="true">
      <span v-for="(label, index) in weekdayLabels" :key="index">{{ label }}</span>
    </div>
    <div class="mini-month__grid">
      <CardButton
        v-for="cell in cells"
        :key="cell.key"
        class="mini-month__day"
        :class="{
          'mini-month__day--muted': !cell.inMonth,
          'mini-month__day--today': cell.isToday,
          'mini-month__day--selected': cell.isSelected,
        }"
        :title="cell.day.toLocaleDateString(locale)"
        @click="emit('select', cell.day)"
      >
        {{ cell.day.getDate() }}
      </CardButton>
    </div>
  </div>
</template>

<style scoped>
.mini-month {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.mini-month__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 4px;
}

.mini-month__title {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}

.mini-month__nav {
  display: inline-flex;
  gap: 2px;
}

.mini-month__weekdays,
.mini-month__grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
}

.mini-month__weekdays span {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
  text-align: center;
  padding: 2px 0;
}

.mini-month__day {
  height: 24px;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-2xs);
  font-variant-numeric: tabular-nums;
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.mini-month__day:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.mini-month__day--muted {
  color: var(--text-tertiary);
  opacity: 0.6;
}

.mini-month__day--today {
  color: var(--accent-primary);
  font-weight: var(--weight-bold);
}

.mini-month__day--selected {
  background: var(--accent-primary);
  color: var(--text-on-accent);
  opacity: 1;
}

.mini-month__day--selected:hover {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}
</style>
