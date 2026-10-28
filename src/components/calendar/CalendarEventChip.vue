<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed } from 'vue';
import { formatTime, type CalendarItem } from '@/utils/calendarGrid';
import type { AnchorBox } from '@/utils/popoverPlacement';

const props = withDefaults(
  defineProps<{
    item: CalendarItem;
    /** Hide the start time (all-day lanes, continuation days). */
    showTime?: boolean;
  }>(),
  { showTime: true },
);

const emit = defineEmits<{
  select: [item: CalendarItem, anchor: AnchorBox];
}>();

const timeLabel = computed(() =>
  props.showTime && !props.item.allDay ? formatTime(props.item.start) : '',
);

function onClick(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  emit('select', props.item, rect);
}
</script>

<template>
  <CardButton
    class="event-chip"
    :class="[`event-chip--${item.status}`, { 'event-chip--span': !timeLabel }]"
    :style="item.color ? { '--chip-accent': item.color } : undefined"
    :title="item.title"
    @click.stop="onClick"
  >
    <!-- A timed event is a dot and its time; only what spans the day (all-day,
         or a later day of a longer event) takes a tinted bar. A coloured
         left border on a rounded chip bent with the corners. -->
    <span v-if="timeLabel" class="event-chip__dot" aria-hidden="true" />
    <span v-if="timeLabel" class="event-chip__time">{{ timeLabel }}</span>
    <span class="event-chip__title">{{ item.title }}</span>
  </CardButton>
</template>

<style scoped>
.event-chip {
  /* The source calendar's colour; an invite's answer overrides it below. */
  --chip-tone: var(--chip-accent, var(--accent-primary));
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  min-width: 0;
  border: 0;
  flex-shrink: 0;
  border-radius: var(--radius-xs);
  padding: var(--space-2xs) var(--space-xs);
  background: transparent;
  color: var(--text-primary);
  font-size: var(--font-size-2xs);
  line-height: 1.5;
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.event-chip:hover {
  background: color-mix(in srgb, var(--chip-tone) 14%, transparent);
}

.event-chip:not(.event-chip--span) {
  /* No fill to pad: the dot starts at the chip's edge and the title keeps
     the width a bar and its inset used to take. */
  gap: 3px;
  padding-inline: 2px;
}

.event-chip--span {
  background: color-mix(in srgb, var(--chip-tone) 20%, transparent);
}

.event-chip--span:hover {
  background: color-mix(in srgb, var(--chip-tone) 30%, transparent);
}

.event-chip__dot {
  flex: none;
  width: 6px;
  height: 6px;
  border-radius: var(--radius-full);
  background: var(--chip-tone);
}

.event-chip__time {
  flex-shrink: 0;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

/* One line in every lane, all-day included: a wrapped chip makes its row
   taller than its neighbours. The full title is in the tooltip. */
.event-chip__title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-weight: var(--weight-semibold);
}

.event-chip--accepted {
  --chip-tone: var(--accent-green);
}

.event-chip--tentative {
  --chip-tone: var(--accent-amber);
}

.event-chip--needs-action {
  --chip-tone: var(--accent-purple);
}

.event-chip--declined,
.event-chip--cancelled {
  --chip-tone: var(--text-tertiary);
  color: var(--text-tertiary);
}

.event-chip--declined.event-chip--span,
.event-chip--cancelled.event-chip--span {
  background: var(--bg-raised);
}

.event-chip--declined .event-chip__title,
.event-chip--cancelled .event-chip__title {
  text-decoration: line-through;
}
</style>
