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
    :class="`event-chip--${item.status}`"
    :style="item.color ? { '--chip-accent': item.color } : undefined"
    :title="item.title"
    @click.stop="onClick"
  >
    <span v-if="timeLabel" class="event-chip__time">{{ timeLabel }}</span>
    <span class="event-chip__title">{{ item.title }}</span>
  </CardButton>
</template>

<style scoped>
.event-chip {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  min-width: 0;
  border: 0;
  border-radius: var(--radius-xs);
  padding: 1px 6px;
  /* Own events are tinted by their source calendar; invites override below. */
  background: color-mix(in srgb, var(--chip-accent, var(--accent-primary)) 18%, transparent);
  color: var(--text-primary);
  font-size: var(--font-size-2xs);
  line-height: 1.5;
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.event-chip:hover {
  background: color-mix(in srgb, var(--chip-accent, var(--accent-primary)) 30%, transparent);
}

.event-chip__time {
  flex-shrink: 0;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.event-chip__title {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-weight: var(--weight-semibold);
}

.event-chip--accepted {
  background: color-mix(in srgb, var(--accent-green) 18%, transparent);
}
.event-chip--accepted:hover {
  background: color-mix(in srgb, var(--accent-green) 30%, transparent);
}

.event-chip--tentative {
  background: color-mix(in srgb, var(--accent-amber) 18%, transparent);
}
.event-chip--tentative:hover {
  background: color-mix(in srgb, var(--accent-amber) 30%, transparent);
}

.event-chip--needs-action {
  background: color-mix(in srgb, var(--accent-purple) 18%, transparent);
}
.event-chip--needs-action:hover {
  background: color-mix(in srgb, var(--accent-purple) 30%, transparent);
}

.event-chip--declined,
.event-chip--cancelled {
  background: var(--bg-raised);
  color: var(--text-tertiary);
}

.event-chip--declined .event-chip__title,
.event-chip--cancelled .event-chip__title {
  text-decoration: line-through;
}
</style>
