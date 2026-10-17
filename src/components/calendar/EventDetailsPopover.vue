<script setup lang="ts">
import { computed } from 'vue';
import { isSameDay, type CalendarItem } from '@/utils/calendarGrid';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useI18n } from 'vue-i18n';

const props = defineProps<{
  item: CalendarItem;
}>();

const emit = defineEmits<{
  close: [];
  respond: [item: CalendarItem, response: 'accepted' | 'declined' | 'tentative'];
  edit: [item: CalendarItem];
  remove: [item: CalendarItem];
}>();

const { d, t } = useI18n();

const statusLabel = computed(() => {
  switch (props.item.status) {
    case 'accepted':
      return t('calendar.status.accepted');
    case 'declined':
      return t('calendar.status.declined');
    case 'tentative':
      return t('calendar.status.tentative');
    case 'needs-action':
      return t('calendar.status.needsAction');
    case 'cancelled':
      return t('calendar.status.cancelled');
    default:
      if (props.item.source === 'caldav') return 'GroupWise';
      if (props.item.source === 'google') return t('calendar.status.googleCalendar');
      return t('calendar.status.ownEvent');
  }
});

const canRespond = computed(() => props.item.kind === 'invite' && props.item.status !== 'cancelled');

// The Google mirror is one-way read-only: an edit would be overwritten by the
// next sync and a delete would resurrect. Manage those events in Google.
const canModify = computed(() => props.item.kind === 'event' && props.item.source !== 'google');

const rangeLabel = computed(() => {
  const item = props.item;
  const inclusiveEnd = new Date(item.end.getTime() - 1);
  if (item.allDay) {
    if (isSameDay(item.start, inclusiveEnd)) {
      return t('calendar.range.allDaySingle', { day: d(item.start, 'long') });
    }
    return t('calendar.range.allDayMulti', {
      start: d(item.start, 'date'),
      end: d(inclusiveEnd, 'long'),
    });
  }
  if (isSameDay(item.start, inclusiveEnd)) {
    return t('calendar.range.sameDay', {
      day: d(item.start, 'long'),
      start: d(item.start, 'time'),
      end: d(item.end, 'time'),
    });
  }
  return t('calendar.range.span', {
    start: t('calendar.range.dayTime', { day: d(item.start, 'long'), time: d(item.start, 'time') }),
    end: t('calendar.range.dayTime', { day: d(item.end, 'long'), time: d(item.end, 'time') }),
  });
});
</script>

<template>
  <div class="event-card">
    <header class="event-card__header">
      <span class="event-card__status" :class="`event-card__status--${item.status}`">
        {{ statusLabel }}
      </span>
      <span class="event-card__tools">
        <template v-if="canModify">
          <IconButton size="sm" :title="$t('calendar.event.edit')" @click="emit('edit', item)">✎</IconButton>
          <IconButton
            size="sm"
            tone="danger"
            :title="$t('calendar.event.delete')"
            @click="emit('remove', item)"
          >
            🗑
          </IconButton>
        </template>
        <IconButton size="sm" :title="$t('common.close')" @click="emit('close')">✕</IconButton>
      </span>
    </header>

    <h3 class="event-card__title" :class="{ cancelled: item.status === 'cancelled' }">
      {{ item.title }}
    </h3>
    <p class="event-card__when">{{ rangeLabel }}</p>

    <dl v-if="item.location || item.organizer || item.attendees.length > 0" class="event-card__meta">
      <template v-if="item.location">
        <dt>{{ $t('calendar.event.location') }}</dt>
        <dd>{{ item.location }}</dd>
      </template>
      <template v-if="item.organizer">
        <dt>{{ $t('calendar.event.organizer') }}</dt>
        <dd>{{ item.organizer }}</dd>
      </template>
      <template v-if="item.attendees.length > 0">
        <dt>{{ $t('calendar.event.attendees') }}</dt>
        <dd>{{ item.attendees.join(', ') }}</dd>
      </template>
    </dl>

    <p v-if="item.description" class="event-card__description">{{ item.description }}</p>

    <footer v-if="canRespond" class="event-card__actions">
      <AppButton variant="primary" size="sm" @click="emit('respond', item, 'accepted')">
        {{ $t('calendar.rsvp.accept') }}
      </AppButton>
      <AppButton variant="soft" size="sm" @click="emit('respond', item, 'tentative')">
        {{ $t('calendar.rsvp.maybe') }}
      </AppButton>
      <AppButton variant="danger" size="sm" @click="emit('respond', item, 'declined')">
        {{ $t('calendar.rsvp.decline') }}
      </AppButton>
    </footer>
  </div>
</template>

<style scoped>
.event-card {
  display: flex;
  flex-direction: column;
  padding: 12px 14px 14px;
}

.event-card__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 8px;
}

.event-card__tools {
  display: inline-flex;
  align-items: center;
  gap: 2px;
}

.event-card__status {
  padding: 2px 10px;
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  white-space: nowrap;
}

.event-card__status--accepted {
  background: color-mix(in srgb, var(--accent-green) 18%, transparent);
  color: var(--accent-green);
}

.event-card__status--tentative {
  background: color-mix(in srgb, var(--accent-amber) 18%, transparent);
  color: var(--accent-amber);
}

.event-card__status--needs-action {
  background: color-mix(in srgb, var(--accent-purple) 18%, transparent);
  color: var(--accent-purple);
}

.event-card__status--declined,
.event-card__status--cancelled {
  background: color-mix(in srgb, var(--accent-red) 14%, transparent);
  color: var(--accent-red);
}

.event-card__status--own {
  background: color-mix(in srgb, var(--accent-primary) 16%, transparent);
  color: var(--accent-primary);
}

.event-card__title {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  letter-spacing: -0.01em;
}

.event-card__title.cancelled {
  text-decoration: line-through;
  color: var(--text-secondary);
}

.event-card__when {
  margin: 2px 0 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.event-card__meta {
  display: grid;
  grid-template-columns: 84px minmax(0, 1fr);
  gap: 4px 10px;
  margin: 10px 0 0;
  font-size: var(--font-size-sm);
}

.event-card__meta dt {
  color: var(--text-tertiary);
}

.event-card__meta dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
  color: var(--text-primary);
}

.event-card__description {
  margin: 10px 0 0;
  max-height: 140px;
  overflow: auto;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
  white-space: pre-wrap;
}

.event-card__actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
  margin-top: 12px;
}
</style>
