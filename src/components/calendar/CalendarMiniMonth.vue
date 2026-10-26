<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { computed, ref, watch } from 'vue';
import {
  addMonths,
  isSameDay,
  monthGridDays,
  weekdayLabels,
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

const weekdayHeaders = computed(() => weekdayLabels(locale.value, 'narrow'));

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
        <IconButton :title="$t('calendar.nav.previousMonth')" @click="cursor = addMonths(cursor, -1)">
          <AppIcon><path d="m10 3-5 5 5 5" /></AppIcon>
        </IconButton>
        <IconButton :title="$t('calendar.nav.nextMonth')" @click="cursor = addMonths(cursor, 1)">
          <AppIcon><path d="m6 3 5 5-5 5" /></AppIcon>
        </IconButton>
      </span>
    </header>
    <div class="mini-month__weekdays" aria-hidden="true">
      <span v-for="(label, index) in weekdayHeaders" :key="index">{{ label }}</span>
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

/* The 32px targets reach past the header line instead of pushing the grid
   down; the outer button's glyph ends on the grid's right edge. */
.mini-month__nav {
  display: inline-flex;
  margin-block: calc(-1 * var(--space-xs));
  margin-right: calc(-1 * var(--space-sm));
}

.mini-month__weekdays,
.mini-month__grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
}

.mini-month__weekdays {
  padding-bottom: var(--space-2xs);
  border-bottom: 1px solid var(--border-color-muted);
  margin-bottom: var(--space-2xs);
}

.mini-month__weekdays span {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-medium);
  text-align: center;
  padding: var(--space-xs) 0;
}

/*
 * A circle (28px where the column allows) centred in its column, so the number sits in the middle of
 * the today/selected fill and lines up under the centred weekday header.
 * CardButton is a left-aligned flex box by default; both are undone here.
 */
.mini-month__day {
  justify-self: center;
  justify-content: center;
  align-items: center;
  width: min(28px, 100%);
  aspect-ratio: 1;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-primary);
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

/* The neighbouring months are context, not targets to notice: plain,
   dimmed numbers, no fill — the main grid shades them, the mini month does
   not have the room to. */
.mini-month__day--muted {
  color: color-mix(in srgb, var(--text-tertiary) 70%, transparent);
}

.mini-month__day--today {
  color: var(--text-accent);
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
