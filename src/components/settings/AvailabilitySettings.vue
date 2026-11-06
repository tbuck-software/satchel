<script setup lang="ts">
/**
 * Working hours and the calendars that make you busy. Both feed "Insert my
 * availability" in the composer and "Next time everyone is free" in the
 * event editor; neither changes anything on a server.
 */
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import FilterPill from '@/components/ui/FilterPill.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { useCalendarUiStore } from '@/stores/calendarUi';
import { minuteToTimeInput, timeInputToMinute } from '@/utils/workingHours';

defineProps<{
  /** The calendars the calendar settings list, with their keys and names. */
  sources: Array<{ key: string; label: string }>;
}>();

const { t, locale } = useI18n();
const calendarUi = useCalendarUiStore();

const startInput = ref(minuteToTimeInput(calendarUi.workingHours.startMinute));
const endInput = ref(minuteToTimeInput(calendarUi.workingHours.endMinute));
const error = ref('');

watch(
  () => calendarUi.workingHours,
  (hours) => {
    startInput.value = minuteToTimeInput(hours.startMinute);
    endInput.value = minuteToTimeInput(hours.endMinute);
  },
);

const weekdays = computed(() => [1, 2, 3, 4, 5, 6, 7].map((day) => ({
  day,
  // 2024-01-01 was a Monday; ISO day 1 lands on it.
  label: new Date(2024, 0, day).toLocaleDateString(locale.value, { weekday: 'short' }),
  pressed: calendarUi.workingHours.weekdays.includes(day),
})));

function saveHours() {
  const start = timeInputToMinute(startInput.value);
  const end = timeInputToMinute(endInput.value, true);
  if (start === null || end === null || end <= start) {
    error.value = t('availability.settings.invalidHours');
    return;
  }
  error.value = '';
  calendarUi.setWorkingHours({ ...calendarUi.workingHours, startMinute: start, endMinute: end });
}

function toggleDay(day: number) {
  const current = calendarUi.workingHours.weekdays;
  const next = current.includes(day) ? current.filter((entry) => entry !== day) : [...current, day];
  // At least one working day: a week without one has no free time to offer.
  if (next.length === 0) return;
  calendarUi.setWorkingHours({ ...calendarUi.workingHours, weekdays: next });
}
</script>

<template>
  <section class="availability-settings" data-testid="availability-settings">
    <div>
      <h4>{{ $t('availability.settings.title') }}</h4>
      <p class="hint">{{ $t('availability.settings.description') }}</p>
    </div>

    <div class="hours">
      <label class="field">
        <span>{{ $t('availability.settings.workStart') }}</span>
        <input v-model="startInput" type="time" step="900" @change="saveHours" />
      </label>
      <label class="field">
        <span>{{ $t('availability.settings.workEnd') }}</span>
        <input v-model="endInput" type="time" step="900" @change="saveHours" />
      </label>
    </div>
    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <div class="field">
      <span>{{ $t('availability.settings.workDays') }}</span>
      <div class="days" role="group" :aria-label="$t('availability.settings.workDays')">
        <FilterPill
          v-for="entry in weekdays"
          :key="entry.day"
          :label="entry.label"
          :pressed="entry.pressed"
          @click="toggleDay(entry.day)"
        />
      </div>
    </div>

    <div v-if="sources.length" class="field">
      <span>{{ $t('availability.settings.calendars') }}</span>
      <div class="calendars">
        <ToggleSwitch
          v-for="source in sources"
          :key="source.key"
          :model-value="calendarUi.countsForAvailability(source.key)"
          @update:model-value="calendarUi.toggleAvailability(source.key)"
        >{{ source.label }}</ToggleSwitch>
      </div>
    </div>
  </section>
</template>

<style scoped src="./settingsForms.css"></style>

<style scoped>
.availability-settings {
  border: var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  padding: var(--space-md);
}

.hours {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 10rem));
  gap: var(--space-md);
}

.days {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-xs);
}

.calendars {
  display: grid;
  gap: var(--space-xs);
}

.error {
  margin: 0;
  color: var(--accent-red);
  font-size: var(--font-size-sm);
}
</style>
