<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed, onMounted } from 'vue';
import { useInvitesStore } from '@/stores/invites';
import { useCalendarUiStore } from '@/stores/calendarUi';
import {
  collectSources,
  itemFromEvent,
  itemFromInvite,
  type CalendarItem,
} from '@/utils/calendarGrid';
import CalendarMiniMonth from './CalendarMiniMonth.vue';
import { useI18n } from 'vue-i18n';

const invites = useInvitesStore();
const calendarUi = useCalendarUiStore();
const { t } = useI18n();

onMounted(() => {
  // Usually already loaded by the calendar view; harmless when it is.
  void invites.load();
});

const sources = computed(() => {
  const items: CalendarItem[] = [
    ...invites.events.map(itemFromEvent),
    ...invites.items.map(itemFromInvite).filter((item): item is CalendarItem => item !== null),
  ];
  const definitions = invites.sources.map((source) => ({
    key: `${source.source}:${source.remote_id}`,
    label: source.name,
  }));
  return collectSources(items, definitions);
});
</script>

<template>
  <div class="cal-sidebar">
    <CalendarMiniMonth :selected="calendarUi.anchor" @select="calendarUi.setAnchor($event)" />

    <section v-if="sources.length > 0" class="cal-sidebar__sources">
      <h3 class="cal-sidebar__heading">{{ $t('calendar.source.heading') }}</h3>
      <CardButton
        v-for="source in sources"
        :key="source.key"
        class="cal-source"
        :class="{ 'cal-source--hidden': calendarUi.isSourceHidden(source.key) }"
        :title="calendarUi.isSourceHidden(source.key) ? t('calendar.source.show') : t('calendar.source.hide')"
        :aria-pressed="!calendarUi.isSourceHidden(source.key)"
        @click="calendarUi.toggleSource(source.key)"
      >
        <span class="cal-source__dot" :style="{ backgroundColor: source.color }" aria-hidden="true">
          <svg
            v-if="!calendarUi.isSourceHidden(source.key)"
            width="8"
            height="8"
            viewBox="0 0 8 8"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M1.5 4.2l1.8 1.8 3.2-4" />
          </svg>
        </span>
        <span class="cal-source__label">{{ source.label }}</span>
        <span class="cal-source__count">{{ source.count }}</span>
      </CardButton>
    </section>
  </div>
</template>

<style scoped>
.cal-sidebar {
  display: flex;
  flex-direction: column;
  gap: var(--space-lg);
  padding: var(--space-md);
  overflow-y: auto;
}

.cal-sidebar__heading {
  margin: 0 0 var(--space-xs);
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.08em;
}

.cal-sidebar__sources {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.cal-source {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  border: 0;
  border-radius: var(--radius-sm);
  padding: 5px 8px;
  background: transparent;
  cursor: pointer;
  text-align: left;
  transition: background var(--duration-fast) var(--ease-out);
}

.cal-source:hover {
  background: var(--bg-hover);
}

.cal-source__dot {
  display: grid;
  place-items: center;
  flex-shrink: 0;
  width: 14px;
  height: 14px;
  border-radius: var(--radius-xs);
  color: var(--text-on-accent);
}

.cal-source--hidden .cal-source__dot {
  opacity: 0.35;
}

.cal-source__label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}

.cal-source--hidden .cal-source__label {
  color: var(--text-tertiary);
}

.cal-source__count {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-variant-numeric: tabular-nums;
}
</style>
